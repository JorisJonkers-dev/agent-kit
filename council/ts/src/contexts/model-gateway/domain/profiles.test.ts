import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import type { AllowlistEntry } from './allowlist.js'
import { ModelGatewayError, parseDefaults, type GatewayConfig } from './config.js'
import { readNarrowing, type Narrowing } from './narrowing.js'
import { decidePolicy, type DataScope } from './policy.js'
import {
  effectiveSpend,
  effectiveTools,
  holdToProfile,
  listProfiles,
  profileModels,
  requireNarrowing,
  requireProfile,
  resolveProfile,
  plainName,
  runScope,
  toolsJson,
  type ProfileModel,
} from './profiles.js'

const SHIPPED = join(import.meta.dirname, '../../../../../../skills/workstation/model-gateway/defaults.json')

function shipped(): GatewayConfig {
  return parseDefaults(JSON.parse(readFileSync(SHIPPED, 'utf8')) as unknown)
}

/** Three clients and a few models of three families: enough to state any profile. */
function custom(profiles: unknown): GatewayConfig {
  return parseDefaults({
    clients: {
      claude: { binary: 'claude', cost_class: 'subsidised', model_source: 'declared' },
      'pi-vibe': { binary: 'pi', cost_class: 'subsidised', model_source: 'live' },
      vibe: { binary: 'vibe', cost_class: 'metered-personal', model_source: 'declared' },
    },
    models: [
      { client: 'claude', family: 'anthropic', model: 'opus', notes: 'n', tier: 'frontier' },
      { client: 'claude', family: 'anthropic', model: 'haiku', notes: 'n', tier: 'routine' },
      { client: 'pi-vibe', family: 'mistral', model: 'small', notes: 'n', tier: 'trivial' },
      { client: 'pi-vibe', family: 'zhipu', model: 'big', notes: 'n', tier: 'hard' },
      { client: 'vibe', family: 'zhipu', model: 'paid', notes: 'n', tier: 'trivial' },
    ],
    profiles,
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

function refusedConfig(profiles: unknown): string {
  try {
    custom(profiles)
  } catch (error) {
    if (error instanceof ModelGatewayError) return error.message
    throw error
  }
  throw new Error('the profiles were accepted')
}

function repository(families: readonly string[]): DataScope {
  const entry: AllowlistEntry = { code: true, families, history: false }
  return { entry, key: 'github.com/owner/repo', kind: 'repository' }
}

const EVERY_FAMILY = ['zhipu', 'mistral', 'deepseek', 'moonshot', 'alibaba', 'google']

function resolution(config: GatewayConfig, name: string, scope: DataScope, narrow?: Narrowing) {
  return profileModels(config, decidePolicy(config, scope), { client: '', model: '', profile: name, ...(narrow === undefined ? {} : { narrow }) })
}

const keys = (models: readonly ProfileModel[]): string[] => models.map((model) => `${model.client}:${model.model}`)

describe('the shipped profiles', () => {
  it('state the four seed profiles with a deny-all tool scope and a spend cap', () => {
    const config = shipped()

    expect(Object.keys(config.profiles).sort()).toEqual(['cheap-research', 'cross-critic', 'implement-cheap', 'public-free'])
    for (const profile of Object.values(config.profiles)) {
      expect(profile.tools).toEqual({ mcp: [], plugins: [], skills: [] })
      expect(profile.max_spend_usd).toBeGreaterThan(0)
    }
    expect(config.profiles['cross-critic']).toMatchObject({ distinct_families: 2, exclude_writer_family: true })
    expect(config.profiles['implement-cheap']?.ladder).toHaveLength(4)
  })

  it('leave a model with no repository, none for a private or nedap repository, and none on a public route for any', () => {
    const config = shipped()
    const writer: Narrowing = { writer_family: 'mistral' }
    for (const name of ['cheap-research', 'cross-critic', 'implement-cheap', 'public-free']) {
      const narrow = name === 'cross-critic' ? writer : undefined
      expect(resolution(config, name, { kind: 'outside' }, narrow).error, `${name} outside`).toBeUndefined()

      const priv = resolution(config, name, repository(['anthropic']), narrow)
      expect(priv.models, `${name} private`).toEqual([])
      expect(priv.error).toContain(`profile ${name} has no allowed model`)

      const nedap = resolution(config, name, { kind: 'nedap' }, narrow)
      expect(nedap.models, `${name} nedap`).toEqual([])
      expect(nedap.error).toContain('a nedap repository')

      const publicRepo = resolution(config, name, repository(EVERY_FAMILY), narrow)
      expect(publicRepo.models.length === 0, `${name} public`).toBe(name === 'public-free')
    }
  })

  it('list a profile cheapest first and let a request narrow it', () => {
    const config = shipped()
    const scope = repository(EVERY_FAMILY)

    expect(keys(resolution(config, 'implement-cheap', scope).models)).toEqual([
      'pi-vibe:deepseek-v4.1-flash',
      'pi-vibe:glm-5.3-flash',
      'pi-vibe:mistral-large-4',
      'pi-vibe:glm-5.3',
    ])
    expect(keys(resolution(config, 'implement-cheap', scope, { models: ['pi-vibe:glm-5.3'] }).models)).toEqual(['pi-vibe:glm-5.3'])
  })

  it('leave out the writer family and need two families', () => {
    const config = shipped()
    const scope = repository(EVERY_FAMILY)

    const left = resolution(config, 'cross-critic', scope, { writer_family: 'mistral' })
    expect(left.models.every((model) => model.family !== 'mistral')).toBe(true)
    expect(resolution(config, 'cross-critic', scope).error).toContain('narrow.writer_family')
    const onlyZhipu = repository(['zhipu'])
    expect(resolution(config, 'cross-critic', onlyZhipu, { writer_family: 'zhipu' }).error).toContain("once the writer's family zhipu is left out")
    expect(resolution(config, 'cross-critic', onlyZhipu, { writer_family: 'mistral' }).error).toContain(
      'needs 2 distinct families but only 1 remain (zhipu)',
    )
  })
})

describe('parseDefaults: profiles', () => {
  it('reads every field and the tool scope, deny-all where a kind is left out', () => {
    const config = custom({
      all: {
        description: 'd',
        distinct_families: 2,
        exclude_writer_family: false,
        ladder: ['pi-vibe:small', 'pi-vibe:big'],
        max_spend_usd: 1.5,
        models: ['pi-vibe:small', 'pi-vibe:big'],
        pin: 'pi-vibe:big',
        tools: { mcp: 'any', skills: ['review'] },
      },
      bare: { description: 'd', models: ['pi-vibe:small'] },
    })

    expect(config.profiles.all?.tools).toEqual({ mcp: 'any', plugins: [], skills: ['review'] })
    expect(config.profiles.all).toMatchObject({ exclude_writer_family: false, max_spend_usd: 1.5, pin: 'pi-vibe:big' })
    expect(config.profiles.bare?.tools).toEqual({ mcp: [], plugins: [], skills: [] })
    expect(custom(undefined).profiles).toEqual({})
  })

  it('refuses a profile that is malformed, saying what is wrong', () => {
    const one = (body: unknown): unknown => ({ p: body })
    const ok = { description: 'd', models: ['pi-vibe:small'] }
    expect(refusedConfig([])).toBe('profiles must be an object keyed by profile name')
    expect(refusedConfig({ Bad_Name: {} })).toContain('a profile name is lowercase letters, digits and hyphens')
    expect(refusedConfig(one(1))).toBe('profiles.p must be an object')
    expect(refusedConfig(one({ ...ok, surprise: 1 }))).toContain('profiles.p.surprise is not a field a profile has')
    expect(refusedConfig(one({ models: ok.models }))).toBe('profiles.p.description must be a non-empty string')
    expect(refusedConfig(one({ description: 'd' }))).toBe('profiles.p.models must be a list of non-empty strings')
    expect(refusedConfig(one({ description: 'd', models: [''] }))).toBe('profiles.p.models must be a list of non-empty strings')
    expect(refusedConfig(one({ description: 'd', models: [] }))).toBe('profiles.p.models must not be empty')
    expect(refusedConfig(one({ description: 'd', models: ['pi-vibe:small', 'pi-vibe:small'] }))).toBe('profiles.p.models must not repeat an entry')
    expect(refusedConfig(one({ description: 'd', models: ['pi-vibe:nothing'] }))).toBe(
      'profiles.p.models names pi-vibe:nothing, which is no model of the catalogue',
    )
    expect(refusedConfig(one({ ...ok, pin: 'pi-vibe:big' }))).toBe("profiles.p.pin must name one of the profile's models")
    expect(refusedConfig(one({ ...ok, ladder: ['pi-vibe:big'] }))).toBe("profiles.p.ladder must name one of the profile's models")
    expect(refusedConfig(one({ ...ok, distinct_families: 0 }))).toBe('profiles.p.distinct_families must be a whole number of at least 1')
    expect(refusedConfig(one({ ...ok, exclude_writer_family: 1 }))).toBe('profiles.p.exclude_writer_family must be true or false')
    expect(refusedConfig(one({ ...ok, max_spend_usd: 0 }))).toBe('profiles.p.max_spend_usd must be a number above 0')
    expect(refusedConfig(one({ ...ok, tools: 1 }))).toBe('profiles.p.tools must be an object')
    expect(refusedConfig(one({ ...ok, tools: { shell: [] } }))).toContain('profiles.p.tools.shell is not a kind of tool')
    expect(refusedConfig(one({ ...ok, tools: { mcp: [1] } }))).toBe('profiles.p.tools.mcp must be a list of non-empty strings')
  })
})

describe('readNarrowing', () => {
  it('reads every field, and nothing when the request narrows nothing', () => {
    expect(readNarrowing(undefined)).toBeUndefined()
    expect(
      readNarrowing({ max_spend_usd: 0.1, mcp: [], models: ['a:b'], plugins: ['p'], skills: ['s'], writer_family: 'zhipu' }),
    ).toEqual({ max_spend_usd: 0.1, mcp: [], models: ['a:b'], plugins: ['p'], skills: ['s'], writer_family: 'zhipu' })
  })

  it('refuses what is not a narrowing', () => {
    const message = (raw: unknown): string => {
      try {
        readNarrowing(raw)
      } catch (error) {
        return (error as Error).message
      }
      return 'accepted'
    }
    expect(message('x')).toBe('narrow must be an object')
    expect(message({ surprise: 1 })).toContain('narrow.surprise is not a field')
    expect(message({ models: [] })).toBe('narrow.models must be a non-empty list of non-empty strings')
    expect(message({ mcp: 'x' })).toBe('narrow.mcp must be a list of non-empty strings')
    expect(message({ skills: [''] })).toBe('narrow.skills must be a list of non-empty strings')
    expect(message({ max_spend_usd: 0 })).toBe('narrow.max_spend_usd must be a number above 0')
    expect(message({ writer_family: '' })).toBe('narrow.writer_family must be a non-empty string')
  })
})

describe('resolving a profile', () => {
  it('allows only the pinned model, and says so when the policy leaves it out', () => {
    const config = custom({ pinned: { description: 'd', models: ['pi-vibe:small', 'pi-vibe:big'], pin: 'pi-vibe:big' } })

    expect(keys(resolution(config, 'pinned', repository(['zhipu', 'mistral'])).models)).toEqual(['pi-vibe:big'])
    expect(resolution(config, 'pinned', repository(['mistral'])).error).toContain('(pinned to pi-vibe:big)')
    expect(resolution(config, 'pinned', repository(['zhipu', 'mistral']), { models: ['pi-vibe:small'] }).models).toEqual([])
  })

  it('picks by cost class, then tier, then the profile order', () => {
    const config = custom({ mixed: { description: 'd', models: ['vibe:paid', 'claude:opus', 'pi-vibe:big', 'claude:haiku', 'pi-vibe:small'] } })

    expect(keys(resolution(config, 'mixed', repository(['zhipu', 'mistral', 'anthropic'])).models)).toEqual([
      'pi-vibe:small',
      'claude:haiku',
      'pi-vibe:big',
      'claude:opus',
      'vibe:paid',
    ])
    const profile = requireProfile(config, 'mixed')
    const model = (name: string, tier: ProfileModel['tier']): ProfileModel => ({
      client: 'pi-vibe',
      cost_class: 'subsidised',
      family: 'mistral',
      model: name,
      tier,
    })
    const sorted = resolveProfile(profile, [model('big', null), model('small', 'trivial')], 'reason', undefined, true)
    expect(keys(sorted.models)).toEqual(['pi-vibe:small', 'pi-vibe:big'])
  })

  it('says which profiles the defaults state when one is not', () => {
    expect(() => requireProfile(shipped(), 'nowhere')).toThrow(
      'no delegation profile nowhere: the defaults state cheap-research, cross-critic, implement-cheap, public-free',
    )
    expect(() => requireProfile(custom(undefined), 'x')).toThrow('the defaults state none')
  })

  it('narrows tools and spend and never widens either', () => {
    const config = custom({
      grants: { description: 'd', max_spend_usd: 1, models: ['pi-vibe:small'], tools: { mcp: ['github'], skills: ['a', 'b'] } },
      loose: { description: 'd', models: ['pi-vibe:small'], tools: { mcp: 'any' } },
    })
    const grants = requireProfile(config, 'grants')
    const loose = requireProfile(config, 'loose')

    expect(effectiveTools(grants, { skills: ['a'] })).toEqual({ mcp: ['github'], plugins: [], skills: ['a'] })
    expect(effectiveSpend(grants, { max_spend_usd: 0.5 })).toBe(0.5)
    expect(effectiveSpend(grants, { max_spend_usd: 3 })).toBe(1)
    expect(effectiveSpend(loose, undefined)).toBeUndefined()
    expect(effectiveSpend(loose, { max_spend_usd: 2 })).toBe(2)
    expect(() => { requireNarrowing(grants, undefined); }).not.toThrow()
    expect(() => { requireNarrowing(grants, { skills: ['c'] }); }).toThrow(
      'narrow.skills names c, which profile grants does not grant (its skills scope is a, b)',
    )
    expect(() => { requireNarrowing(grants, { plugins: ['p'] }); }).toThrow('(its plugins scope is deny-all)')
    expect(() => { requireNarrowing(grants, { max_spend_usd: 2 }); }).toThrow("is above profile grants's cap of 1")
    expect(() => { requireNarrowing(grants, { models: ['pi-vibe:big'] }); }).toThrow('narrow.models names pi-vibe:big')
    expect(() => { requireNarrowing(loose, { mcp: ['anything'] }); }).not.toThrow()
  })
})

describe('holding a run to its profile', () => {
  const config = custom({
    deny: { description: 'd', models: ['claude:haiku', 'pi-vibe:small'] },
    grants: { description: 'd', models: ['pi-vibe:small'], tools: { mcp: ['github'], skills: ['a', 'b'] } },
    mixed: { description: 'd', models: ['pi-vibe:small'], tools: { mcp: 'any' } },
    listed: { description: 'd', models: ['pi-vibe:small'], tools: { plugins: ['hello'], skills: ['a', 'b'] } },
    pathy: { description: 'd', models: ['pi-vibe:small'], tools: { plugins: ['../x'], skills: ['a/b'] } },
    open: { description: 'd', models: ['claude:haiku'], tools: { mcp: 'any', plugins: 'any', skills: 'any' } },
  })
  const decision = decidePolicy(config, repository(['anthropic', 'mistral']))
  const hold = (profile: string | undefined, client: string, model: string, isPi: boolean, narrow?: Narrowing): void => { holdToProfile(config, decision, { client, model, ...(narrow === undefined ? {} : { narrow }), ...(profile === undefined ? {} : { profile }) }, isPi); }
  const refusal = (call: () => void): string => {
    try {
      call()
    } catch (error) {
      expect((error as ModelGatewayError).code).toBe('profile-refused')
      return (error as Error).message
    }
    return 'held'
  }

  it('holds a run with no profile to nothing', () => {
    expect(() => { hold(undefined, 'claude', 'opus', false); }).not.toThrow()
  })

  it('refuses a restricted scope on a client that cannot enforce it, and holds the empty one on Pi', () => {
    expect(refusal(() => { hold('deny', 'claude', 'haiku', false); })).toBe(
      'profile deny restricts mcp, plugins, skills (a scope is deny-all unless the profile lists it), and claude cannot enforce a tool scope: only Pi does',
    )
    expect(() => { hold('deny', 'pi-vibe', 'small', true); }).not.toThrow()
    expect(() => { hold('open', 'claude', 'haiku', false); }).not.toThrow()
  })

  it('refuses an MCP server on Pi, with the reason, until the request narrows it away', () => {
    expect(refusal(() => { hold('grants', 'pi-vibe', 'small', true); })).toBe(
      "profile grants grants mcp github, and pi-vibe cannot enforce it: Pi has a built-in MCP client, but a confined launch reaches only the route's proxy port and starts no other program, so a listed server could never connect; mcp stays deny-all on Pi",
    )
    expect(() => { hold('grants', 'pi-vibe', 'small', true, { mcp: [] }); }).not.toThrow()
  })

  it('holds listed skills and plugins on Pi by name, and refuses `any` and anything that is not a plain name', () => {
    expect(() => { hold('listed', 'pi-vibe', 'small', true); }).not.toThrow()
    expect(refusal(() => { hold('mixed', 'pi-vibe', 'small', true); })).toBe(
      'profile mixed leaves mcp open with any, and pi-vibe builds its per-run home only from named items: list the mcp it needs, or leave them out for deny-all',
    )
    expect(refusal(() => { hold('pathy', 'pi-vibe', 'small', true); })).toContain('lists skills entry a/b, which is not a plain name')
    expect(refusal(() => { hold('pathy', 'pi-vibe', 'small', true, { skills: [] }); })).toContain('lists plugins entry ../x, which is not a plain name: a plugin is named by')
    expect(plainName('a.b_c-1')).toBe(true)
    expect(plainName('.hidden')).toBe(false)
  })

  it('names the scope a run holds, in a fixed order, or none without a profile', () => {
    expect(runScope(config, 'listed', { skills: ['a'] })).toEqual({ mcp: [], plugins: ['hello'], skills: ['a'] })
    expect(Object.keys(toolsJson({ skills: ['a'], plugins: [], mcp: [] }))).toEqual(['mcp', 'plugins', 'skills'])
    expect(runScope(config, undefined, undefined)).toBeUndefined()
    expect(runScope(config, 'nowhere', undefined)).toBeUndefined()
  })

  it('refuses a model the profile does not leave, and a widening request', () => {
    expect(refusal(() => { hold('deny', 'pi-vibe', 'big', true); })).toContain('pi-vibe:big is not allowed by profile deny here')
    expect(refusal(() => { hold('deny', 'pi-vibe', 'small', true, { models: ['claude:haiku'] }); })).toContain('is not allowed by profile deny here')
    expect(refusal(() => { hold('grants', 'pi-vibe', 'small', true, { skills: ['c'] }); })).toContain('narrow.skills names c')
    expect(() => { hold('nowhere', 'pi-vibe', 'small', true); }).toThrow('no delegation profile nowhere')
  })

  it('refuses a profile the policy leaves with nothing', () => {
    const nothing = decidePolicy(config, { kind: 'nedap' })
    expect(() => { holdToProfile(config, nothing, { client: 'pi-vibe', model: 'small', profile: 'grants' }, true); }).toThrow(
      'profile grants has no allowed model',
    )
  })
})

describe('listProfiles', () => {
  it('carries every field a profile has, and why one is empty', () => {
    const config = custom({
      full: {
        description: 'd',
        distinct_families: 1,
        exclude_writer_family: true,
        ladder: ['pi-vibe:small'],
        max_spend_usd: 2,
        models: ['pi-vibe:small', 'pi-vibe:big'],
        pin: 'pi-vibe:small',
        tools: { mcp: 'any', plugins: ['p'] },
      },
    })
    const available: ProfileModel[] = [{ client: 'pi-vibe', cost_class: 'subsidised', family: 'mistral', model: 'small', tier: 'trivial' }]

    const [full] = listProfiles(config, available, 'the policy')
    expect(full).toEqual({
      description: 'd',
      distinct_families: 1,
      exclude_writer_family: true,
      ladder: ['pi-vibe:small'],
      max_spend_usd: 2,
      models: [{ client: 'pi-vibe', cost_class: 'subsidised', family: 'mistral', model: 'small', tier: 'trivial' }],
      name: 'full',
      pin: 'pi-vibe:small',
      tools: { mcp: 'any', plugins: ['p'], skills: [] },
    })
    const [empty] = listProfiles(config, [], 'the policy')
    expect(empty?.models).toEqual([])
    expect(empty?.error).toBe(
      'profile full has no allowed model (pinned to pi-vibe:small): none of its 1 models is both usable now and permitted by the data policy (the policy)',
    )
  })

  it('lists the profiles by name', () => {
    expect(listProfiles(shipped(), [], 'x').map((profile) => profile.name)).toEqual(['cheap-research', 'cross-critic', 'implement-cheap', 'public-free'])
  })
})
