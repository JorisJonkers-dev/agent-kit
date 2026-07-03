import { describe, expect, it } from 'vitest'

import {
  ARCHETYPE_CONTEXT_DEFAULTS,
  checkContextPackStaleness,
  createTaskInclusionQuery,
  indexContextPack,
  normalizeArchetype,
  normalizeModelTier,
  parseContextProfile,
  parseEngineSpec,
  parseRoleClass,
  parseSpecSections,
  resolveContextProfile,
  resolveModelMatrix,
  seedContextPackIfAbsent,
  selectContextSlice,
  selectFragments,
  selectSpecSections,
} from './index.js'
import type { ContextFragment, ContextPack, InclusionQuery } from './index.js'

describe('context profiles', () => {
  it('resolves every archetype default', () => {
    expect(resolveContextProfile({ archetype: 'implementer' })).toMatchObject({
      workspace: 'write',
      repo_context: 'targeted',
      skills: 'relevant',
      mcp: 'read',
      network: 'off',
    })
    expect(resolveContextProfile({ archetype: 'researcher' }).network).toBe('on')
    expect(resolveContextProfile({ archetype: 'reviewer' }).repo_context).toBe('targeted')
    expect(resolveContextProfile({ archetype: 'surveyor' }).repo_context).toBe('full')
    expect(resolveContextProfile({ archetype: 'designer-lens' }).mcp).toBe('none')
    expect(resolveContextProfile({ archetype: 'consolidator-judge' }).skills).toBe('named')
    expect(ARCHETYPE_CONTEXT_DEFAULTS.implementer.workspace).toBe('write')
  })

  it('falls back for unknown archetypes and applies named, inline, and explicit overrides', () => {
    expect(normalizeArchetype(undefined)).toBe('implementer')
    expect(normalizeArchetype('unknown')).toBe('implementer')
    expect(parseContextProfile(undefined)).toBeUndefined()
    expect(parseContextProfile('  ')).toBeUndefined()
    expect(parseContextProfile('unknown')).toBeUndefined()

    expect(resolveContextProfile({ context_profile: 'networked' })).toMatchObject({
      profile: 'networked',
      repo_context: 'full',
      network: 'on',
    })
    expect(
      resolveContextProfile({
        context_profile:
          'workspace=none,repo_context=none,skills=all,mcp=write,network=on,ignored=value',
        overrides: { workspace: 'read' },
      }),
    ).toMatchObject({
      workspace: 'read',
      repo_context: 'none',
      skills: 'all',
      mcp: 'write',
      network: 'on',
    })

    expect(
      parseContextProfile(
        'workspace=invalid,repo_context=invalid,skills=invalid,mcp=invalid,network=invalid',
      ),
    ).toEqual({
      workspace: 'write',
      repo_context: 'targeted',
      skills: 'relevant',
      mcp: 'read',
      network: 'off',
    })
  })
})

describe('model matrix', () => {
  it('uses explicit engines without consulting legacy model-shaped data', () => {
    const resolved = resolveModelMatrix({
      engine: { cli: 'codex', model: 'gpt-5.5', label: 'codex:gpt-5.5' },
      model_tier: 'xhigh',
      bulk: true,
    })

    expect(resolved).toEqual({
      engine: { cli: 'codex', model: 'gpt-5.5', label: 'codex:gpt-5.5' },
      model_tier: 'max',
      reason: 'explicit-engine',
    })
  })

  it('routes strict JSON to Claude and bulk work to Codex first', () => {
    expect(resolveModelMatrix({ model_tier: 'hard', strictJson: true })).toMatchObject({
      engine: { cli: 'claude', model: 'sonnet' },
      model_tier: 'strong',
      reason: 'strict-json-claude',
    })
    expect(resolveModelMatrix({ model_tier: 'cheap', workload: 'strict-json' }).engine.cli).toBe(
      'claude',
    )
    expect(resolveModelMatrix({ model_tier: 'high', workload: 'bulk' })).toMatchObject({
      engine: { cli: 'codex', model: 'gpt-5.5' },
      reason: 'bulk-codex-first',
    })
    expect(resolveModelMatrix({ bulk: true }).engine.cli).toBe('codex')
  })

  it('defaults single work to Claude and parses model tiers and engine specs', () => {
    expect(resolveModelMatrix()).toMatchObject({
      engine: { cli: 'claude', model: 'haiku' },
      model_tier: 'standard',
      reason: 'single-claude-default',
    })
    expect(normalizeModelTier('trivial')).toBe('cheap')
    expect(normalizeModelTier('opus')).toBe('max')
    expect(normalizeModelTier('unknown')).toBe('standard')
    expect(parseEngineSpec('claude:opus')).toEqual({
      cli: 'claude',
      model: 'opus',
      label: 'claude:opus',
    })
    expect(() => parseEngineSpec('legacy')).toThrow('engine must be')
  })

  it('aliases claude model names to tiers (haiku/sonnet/opus)', () => {
    expect(normalizeModelTier('haiku')).toBe('cheap')
    expect(normalizeModelTier('sonnet')).toBe('strong')
    expect(normalizeModelTier('opus')).toBe('max')
  })
})

describe('model role-class policy', () => {
  it('routes investigate and cross-critique to codex (default scale)', () => {
    expect(resolveModelMatrix({ roleClass: 'investigate', model_tier: 'strong' })).toMatchObject({
      engine: { cli: 'codex', model: 'gpt-5.5' },
      reason: 'role-class-policy',
    })
    expect(resolveModelMatrix({ roleClass: 'cross-critique' }).engine.cli).toBe('codex')
  })

  it('routes implement to claude and never opus unless explicit (caps max -> sonnet)', () => {
    expect(resolveModelMatrix({ roleClass: 'implement', model_tier: 'cheap' })).toMatchObject({
      engine: { cli: 'claude', model: 'haiku' },
      model_tier: 'cheap',
      reason: 'role-class-policy',
    })
    expect(resolveModelMatrix({ roleClass: 'implement', model_tier: 'max' })).toMatchObject({
      engine: { cli: 'claude', model: 'sonnet' },
      model_tier: 'strong',
    })
  })

  it('routes consolidate-lock to strong claude, opus at the max tier', () => {
    expect(resolveModelMatrix({ roleClass: 'consolidate-lock', model_tier: 'standard' }).engine).toMatchObject({
      cli: 'claude',
      model: 'haiku',
    })
    expect(resolveModelMatrix({ roleClass: 'consolidate-lock', model_tier: 'max' }).engine.model).toBe('opus')
  })

  it('program scale forces codex for every role except consolidate-lock (claude:opus)', () => {
    for (const roleClass of ['investigate', 'cross-critique', 'implement'] as const) {
      expect(resolveModelMatrix({ roleClass, programScale: true, model_tier: 'strong' })).toMatchObject({
        engine: { cli: 'codex', model: 'gpt-5.5' },
        reason: 'program-codex-override',
      })
    }
    expect(resolveModelMatrix({ roleClass: 'consolidate-lock', programScale: true, model_tier: 'cheap' })).toEqual({
      engine: { cli: 'claude', model: 'opus', label: 'claude:opus' },
      model_tier: 'max',
      reason: 'program-consolidate-lock-claude-opus',
    })
  })

  it('keeps strict-json on claude and explicit engine above any role/program policy', () => {
    expect(
      resolveModelMatrix({ roleClass: 'implement', programScale: true, strictJson: true, model_tier: 'strong' }),
    ).toMatchObject({ engine: { cli: 'claude' }, reason: 'strict-json-claude' })
    expect(
      resolveModelMatrix({
        roleClass: 'investigate',
        programScale: true,
        engine: { cli: 'claude', model: 'opus', label: 'claude:opus' },
      }),
    ).toMatchObject({ reason: 'explicit-engine' })
  })

  it('parses role-class kebab canonical + underscore/alias forms, rejects unknown', () => {
    expect(parseRoleClass('cross-critique')).toBe('cross-critique')
    expect(parseRoleClass('cross_critique')).toBe('cross-critique')
    expect(parseRoleClass('consolidate_lock')).toBe('consolidate-lock')
    expect(parseRoleClass('research')).toBe('investigate')
    expect(parseRoleClass(' implement ')).toBe('implement')
    expect(parseRoleClass('nonsense')).toBeUndefined()
    expect(parseRoleClass(undefined)).toBeUndefined()
  })
})

describe('context packs', () => {
  const seedPack: ContextPack = {
    summary: 'Seed pack. Built at 2026-07-02T22:50:34Z from docs.',
    refs: ['README.md', 'domain-glossary'],
    files: ['council/ts/src/domain/context/packs.ts', 'docs/spec.md'],
    snippets: [
      {
        ref: 'domain-glossary',
        path: '.council/context/domain-glossary.md',
        text: 'Common domain terms.',
        content_hash: 'abc123',
      },
      {
        ref: 'review-notes',
        text: 'Review strict JSON output for contract stability.',
      },
    ],
    profile: 'seed-if-absent',
    content_hash: 'pack-hash',
  }

  it('indexes keyed fragments with source and built-at stamps', () => {
    const index = indexContextPack(seedPack, 'pack.json')

    expect(index.source).toBe('pack.json')
    expect(index.built_at).toBe('2026-07-02T22:50:34Z')
    expect(index.profile).toBe('seed-if-absent')
    expect(index.fragments).toHaveLength(6)
    expect(index.by_key.summary).toMatchObject({
      source: 'pack.json',
      built_at: '2026-07-02T22:50:34Z',
      content_hash: 'pack-hash',
    })
    expect(index.by_key['domain-glossary']).toMatchObject({
      kind: 'snippet',
      path: '.council/context/domain-glossary.md',
      content_hash: 'abc123',
    })
  })

  it('reports --check staleness reasons', () => {
    const fresh = indexContextPack(seedPack)
    expect(
      checkContextPackStaleness(fresh, new Date('2026-07-03T00:00:00Z'), 86_400_000),
    ).toEqual({ stale: false, reasons: [] })
    expect(
      checkContextPackStaleness(fresh, new Date('2026-07-05T00:00:00Z'), 86_400_000),
    ).toEqual({ stale: true, reasons: ['expired'] })
    expect(
      checkContextPackStaleness(
        { duplicate_keys: [] },
        new Date('2026-07-03T00:00:00Z'),
        86_400_000,
      ),
    ).toEqual({ stale: true, reasons: ['missing-built-at'] })
    expect(
      checkContextPackStaleness(
        { built_at: 'not-a-date', duplicate_keys: ['domain-glossary'] },
        new Date('2026-07-03T00:00:00Z'),
        86_400_000,
      ),
    ).toEqual({ stale: true, reasons: ['invalid-built-at', 'duplicate-keys'] })
  })

  it('preserves existing packs for seed-if-absent and detects duplicate keys', () => {
    const existing: ContextPack = { summary: 'Local pack', built_at: '2026-07-03T00:00:00Z' }
    const duplicateIndex = indexContextPack({
      summary: 'No stamp',
      refs: ['same'],
      snippets: [{ ref: 'same', text: 'duplicate' }],
    })

    expect(seedContextPackIfAbsent(existing, seedPack)).toBe(existing)
    expect(seedContextPackIfAbsent(undefined, seedPack)).toBe(seedPack)
    expect(duplicateIndex.built_at).toBeUndefined()
    expect(duplicateIndex.profile).toBeUndefined()
    expect(duplicateIndex.duplicate_keys).toEqual(['same'])
    expect(duplicateIndex.by_key.same?.text).toBe('duplicate')
  })

  it('selects per-task slices with the shared inclusion engine', () => {
    const index = indexContextPack(seedPack)
    const task = {
      context_refs: ['review-notes'],
      paths: ['council/ts/src/domain/context'],
      spec_ref: 'api-contract',
    }
    const query = createTaskInclusionQuery(task)
    const slice = selectContextSlice(index, task, { maxFragments: 3 })

    expect(query).toEqual({
      refs: ['review-notes'],
      paths: ['council/ts/src/domain/context'],
      spec_refs: ['api-contract'],
      include_summary: true,
    })
    expect(createTaskInclusionQuery({ paths: [] })).toEqual({
      paths: [],
      include_summary: true,
    })
    expect(slice.keys).toEqual([
      'summary',
      'council/ts/src/domain/context/packs.ts',
      'review-notes',
    ])
    expect(slice.summary).toContain('Review strict JSON output')
  })

  it('selects fragments by term and overlapping paths', () => {
    const fragments: readonly ContextFragment[] = [
      {
        key: 'service',
        kind: 'file',
        source: 'pack',
        path: 'services/api',
        text: 'Service root',
      },
      {
        key: 'contract',
        kind: 'snippet',
        source: 'pack',
        ref: 'contract',
        text: 'Strict JSON contract',
      },
    ]
    const query: InclusionQuery = {
      paths: ['services/api/src/index.ts'],
      terms: ['json'],
    }

    expect(selectFragments(fragments, query).map((fragment) => fragment.key)).toEqual([
      'service',
      'contract',
    ])
  })

  it('uses the same inclusion path for spec-section sharding', () => {
    const sections = parseSpecSections(`
Intro text
# Overview
Shared context
## API Contract
Strict JSON response
`)
    const slice = selectSpecSections(sections, { spec_refs: ['api-contract'] })

    expect(sections).toEqual([
      { ref: 'overview', title: 'Overview', text: 'Shared context' },
      { ref: 'api-contract', title: 'API Contract', text: 'Strict JSON response' },
    ])
    expect(slice.keys).toEqual(['api-contract'])
    expect(slice.summary).toBe('## API Contract\n\nStrict JSON response')
  })
})
