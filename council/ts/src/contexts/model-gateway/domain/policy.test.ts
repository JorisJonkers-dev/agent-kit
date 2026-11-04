import { describe, expect, it } from 'vitest'

import type { AllowlistEntry } from './allowlist.js'
import { parseDefaults, type GatewayConfig } from './config.js'
import { listModels } from './models.js'
import { decidePolicy, permittedModels, type DataScope } from './policy.js'

/** Four clients, one of each cost class plus a configured one, with a model each. */
function config(): GatewayConfig {
  return parseDefaults({
    clients: {
      claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
      codex: { binary: 'codex', cost_class: 'metered-employer', model_source: 'declared' },
      hermes: { binary: 'hermes', cost_class: 'metered-personal', model_source: 'declared' },
      vibe: { binary: 'vibe', cost_class: 'subsidised', model_source: 'declared' },
    },
    models: [
      { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
      { client: 'codex', family: 'openai', model: 'gpt-5.5', notes: 'implementation', tier: 'frontier' },
      { client: 'hermes', family: 'mistral', model: 'hermes-4', notes: 'metered', tier: 'routine' },
      { client: 'vibe', family: 'zhipu', model: 'zai-glm-5-3', notes: 'review', tier: 'hard' },
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
  })
}

function entry(families: readonly string[], code = false, history = false): AllowlistEntry {
  return { code, families, history }
}

/** The permitted client models of a scope, as `<client>:<model>` keys. */
function permitted(scope: DataScope): string[] {
  return decidePolicy(config(), scope)
    .permissions.filter((permission) => permission.permitted)
    .map((permission) => `${permission.client}:${permission.model}`)
}

describe('decidePolicy', () => {
  it('permits only the families the repository entry lists', () => {
    const scope: DataScope = { entry: entry(['anthropic', 'zhipu']), key: 'github.com/acme/widgets', kind: 'repository' }

    expect(permitted(scope)).toEqual(['claude:claude-sonnet-5-5', 'vibe:zai-glm-5-3'])
  })

  it('reports the entry code and history permissions for every model it permits', () => {
    const scope: DataScope = { entry: entry(['anthropic'], true, false), key: 'github.com/acme/widgets', kind: 'repository' }

    const decision = decidePolicy(config(), scope)

    expect(decision.permissions.find((permission) => permission.client === 'claude')).toEqual({
      client: 'claude',
      inline_only: false,
      may_send: { code: true, history: false },
      model: 'claude-sonnet-5-5',
      permitted: true,
    })
    expect(decision.permissions.find((permission) => permission.client === 'codex')).toMatchObject({
      may_send: { code: true, history: false },
      permitted: false,
    })
  })

  it('permits nothing in a repository with a remote but no entry, and says why', () => {
    const scope: DataScope = { entry: undefined, key: 'github.com/acme/widgets', kind: 'repository' }

    const decision = decidePolicy(config(), scope)

    expect(decision.permissions.every((permission) => !permission.permitted)).toBe(true)
    expect(decision.permissions.every((permission) => !permission.may_send.code && !permission.may_send.history)).toBe(true)
    expect(decision.reason).toBe('github.com/acme/widgets has no allowlist entry, so nothing is sent')
  })

  it('permits nothing in a repository with no remote, and says why', () => {
    const decision = decidePolicy(config(), { kind: 'no-remote' })

    expect(decision.permissions.every((permission) => !permission.permitted)).toBe(true)
    expect(decision.reason).toBe('the repository has no remote, so nothing is sent')
  })

  it('permits only the Codex and Claude Code clients in a nedap repository, not their families served elsewhere', () => {
    const elsewhere = parseDefaults({
      clients: {
        agy: { binary: 'agy', cost_class: 'subsidised', model_source: 'declared' },
        claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
        codex: { binary: 'codex', cost_class: 'metered-employer', model_source: 'declared' },
        opencode: { binary: 'opencode', cost_class: 'metered-personal', model_source: 'declared' },
      },
      models: [
        { client: 'agy', family: 'anthropic', model: 'gemini-claude', notes: 'Claude through Antigravity', tier: 'hard' },
        { client: 'claude', family: 'anthropic', model: 'claude-sonnet-5-5', notes: 'general', tier: 'hard' },
        { client: 'codex', family: 'openai', model: 'gpt-5.5', notes: 'implementation', tier: 'frontier' },
        { client: 'opencode', family: 'openai', model: 'gpt-5.5', notes: 'OpenAI through OpenCode', tier: 'frontier' },
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
    })

    const decision = decidePolicy(elsewhere, { kind: 'nedap' })

    expect(decision.permissions.filter((permission) => permission.permitted).map((permission) => permission.client)).toEqual([
      'claude',
      'codex',
    ])
    expect(decision.permissions.find((permission) => permission.client === 'agy')).toMatchObject({ permitted: false })
    expect(decision.permissions.find((permission) => permission.client === 'opencode')).toMatchObject({ permitted: false })
    expect(decision.permissions.find((permission) => permission.client === 'codex')?.may_send).toEqual({ code: true, history: true })
  })

  it('permits only Codex and Claude Code in a nedap repository, whatever an entry would say', () => {
    const decision = decidePolicy(config(), { kind: 'nedap' })

    expect(permitted({ kind: 'nedap' })).toEqual(['claude:claude-sonnet-5-5', 'codex:gpt-5.5'])
    expect(decision.permissions.every((permission) => permission.may_send.code && permission.may_send.history)).toBe(true)
    expect(decision.reason).toBe('a nedap repository: only Codex and Claude Code, with code and history')
  })

  it('permits only subsidised and metered-personal models outside any repository, inline only', () => {
    const decision = decidePolicy(config(), { kind: 'outside' })

    expect(permitted({ kind: 'outside' })).toEqual(['claude:claude-sonnet-5-5', 'hermes:hermes-4', 'vibe:zai-glm-5-3'])
    expect(decision.permissions.every((permission) => permission.inline_only)).toBe(true)
    expect(decision.permissions.every((permission) => !permission.may_send.code && !permission.may_send.history)).toBe(true)
    expect(decision.reason).toBe('no repository: only inline content, to subsidised and metered-personal clients')
  })

  it('leaves out a model the owner disabled', () => {
    const disabled = { ...config(), disabled: ['claude:claude-sonnet-5-5'] }

    const decision = decidePolicy(disabled, { kind: 'nedap' })

    expect(decision.permissions.map((permission) => permission.client)).toEqual(['codex', 'hermes', 'vibe'])
  })

  it('permits nothing for a model whose client the configuration does not name', () => {
    const orphan = {
      ...config(),
      models: [{ client: 'ghost', family: 'mistral', model: 'ghost-1', notes: 'nowhere', tier: 'routine' as const }],
    }

    expect(decidePolicy(orphan, { kind: 'outside' }).permissions).toEqual([
      { client: 'ghost', inline_only: true, may_send: { code: false, history: false }, model: 'ghost-1', permitted: false },
    ])
  })

  it('says an entry that lists no family allows none', () => {
    const scope: DataScope = { entry: entry([]), key: 'github.com/acme/widgets', kind: 'repository' }

    expect(decidePolicy(config(), scope).reason).toBe('github.com/acme/widgets: the entry allows no family')
  })
})

describe('permittedModels', () => {
  it('keeps the listed models the policy permits, with what each may carry, and the missing clients', () => {
    const listing = listModels(config(), new Set(['claude', 'codex']), new Map())
    const decision = decidePolicy(config(), { entry: entry(['anthropic'], true, true), key: 'github.com/acme/widgets', kind: 'repository' })

    expect(permittedModels(listing, decision)).toEqual({
      missing: [
        { client: 'hermes', reason: 'not installed: hermes is not on PATH' },
        { client: 'vibe', reason: 'not installed: vibe is not on PATH' },
      ],
      models: [
        {
          client: 'claude',
          cost_class: 'subsidised',
          family: 'anthropic',
          inline_only: false,
          may_send: { code: true, history: true },
          model: 'claude-sonnet-5-5',
          notes: 'general',
          source: 'declared',
          tier: 'hard',
        },
      ],
      skipped: [],
    })
  })

  it('leaves out a listed model the policy does not decide about', () => {
    const listing = listModels(config(), new Set(['claude']), new Map())
    const decision = decidePolicy(config(), { kind: 'nedap' })

    expect(permittedModels(listing, { permissions: [], reason: 'nothing' }).models).toEqual([])
    expect(permittedModels(listing, decision).models.map((model) => model.client)).toEqual(['claude'])
  })
})
