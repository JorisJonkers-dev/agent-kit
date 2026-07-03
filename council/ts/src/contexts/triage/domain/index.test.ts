import { describe, expect, it } from 'vitest'

import {
  classifyTriage,
  loadRoutingMatrix,
  matchesCondition,
  parseRoutingMatrix,
  routingMatrix,
  type RoutingMatrix,
  type TriageInput,
} from './index.js'

const trivialInput: TriageInput = {
  size: 'trivial',
  landscape: 'brownfield',
  kind: 'ui-tweak',
  risk: 'low',
  clarity: 'clear',
  parallelism: 'none',
}

describe('triage routing', () => {
  it('loads a schema-pinned routing matrix from the data file', () => {
    expect(loadRoutingMatrix()).toMatchObject({
      $schema: './routing-matrix.schema.json',
      schemaVersion: 1,
    })
    expect(routingMatrix.$schema).toBe('./routing-matrix.schema.json')
    expect(routingMatrix.schemaVersion).toBe(1)
    expect(routingMatrix.useCaseScores).toHaveLength(12)
  })

  it('rejects an unpinned matrix', () => {
    expect(() => parseRoutingMatrix({ schemaVersion: 2 })).toThrow('schema version 1')
  })

  it('rejects a matrix without core arrays', () => {
    expect(() => parseRoutingMatrix({ $schema: './routing-matrix.schema.json', schemaVersion: 1 })).toThrow(
      'profiles, rules, and stage adjustments',
    )
  })

  it('routes trivial clear work to direct without authorizing plan-time workers', () => {
    const verdict = classifyTriage(trivialInput)

    expect(verdict.route).toBe('direct')
    expect(verdict.matchedRuleId).toBe('trivial-clear-direct')
    expect(verdict.candidateRoutes).toEqual(['direct', 'delta'])
    expect(verdict.plan).toEqual({
      dagShape: 'single-minimal-task',
      executesWorkers: false,
      directWorkerPolicy: 'never-during-plan',
    })
    expect(verdict.stageTiers.tasking).toBe('haiku')
  })

  it('routes critical hotfixes to direct even when other program signals exist', () => {
    const verdict = classifyTriage({
      ...trivialInput,
      size: 'large',
      kind: 'hotfix',
      risk: 'critical',
      parallelism: 'high',
    })

    expect(verdict.route).toBe('direct')
    expect(verdict.candidateRoutes).toEqual(['direct', 'program', 'delta'])
    expect(verdict.stageTiers.plan).toBe('opus')
    expect(verdict.reasons).toContain('High risk raises planning and verification tiers.')
  })

  it('routes unclear prototypes to direct with stronger exploration tiers', () => {
    const verdict = classifyTriage({
      ...trivialInput,
      size: 'small',
      kind: 'prototype',
      risk: 'medium',
      clarity: 'unclear',
    })

    expect(verdict.route).toBe('direct')
    expect(verdict.stageTiers.grill).toBe('opus')
    expect(verdict.stageTiers.survey).toBe('sonnet')
  })

  it('routes high-parallel work to program', () => {
    const verdict = classifyTriage({
      ...trivialInput,
      size: 'medium',
      kind: 'feature',
      risk: 'medium',
      parallelism: 'high',
    })

    expect(verdict.route).toBe('program')
    expect(verdict.plan.dagShape).toBe('parallel-program-dag')
  })

  it('routes program-sized work to program', () => {
    expect(classifyTriage({ ...trivialInput, size: 'program', parallelism: 'none' }).matchedRuleId).toBe(
      'program-scale',
    )
  })

  it('routes large high-risk work to program', () => {
    expect(classifyTriage({ ...trivialInput, size: 'large', risk: 'high' }).matchedRuleId).toBe(
      'large-high-risk-program',
    )
  })

  it('routes brownfield modification work to delta', () => {
    const verdict = classifyTriage({ ...trivialInput, size: 'medium', kind: 'refactor', risk: 'medium' })

    expect(verdict.route).toBe('delta')
    expect(verdict.useCaseRefs).toContain('medium-refactor')
  })

  it('routes greenfield work to full', () => {
    expect(classifyTriage({ ...trivialInput, landscape: 'greenfield', kind: 'feature' }).matchedRuleId).toBe(
      'greenfield-full',
    )
  })

  it('routes architecture-heavy brownfield work to full before brownfield fallback', () => {
    expect(classifyTriage({ ...trivialInput, kind: 'api', size: 'medium' }).matchedRuleId).toBe('architecture-full')
  })

  it('uses the brownfield fallback before the full fallback', () => {
    expect(classifyTriage({ ...trivialInput, kind: 'hotfix', risk: 'medium' }).matchedRuleId).toBe('fallback-delta')
  })

  it('uses the full fallback for a custom matrix with only fallback data', () => {
    const fallbackRule = routingMatrix.routeRules.at(-1)
    expect(fallbackRule).toBeDefined()
    const matrix: RoutingMatrix = {
      ...routingMatrix,
      routeRules: fallbackRule === undefined ? [] : [fallbackRule],
    }

    expect(classifyTriage({ ...trivialInput, landscape: 'greenfield' }, matrix).matchedRuleId).toBe('fallback-full')
  })

  it('raises tiers for unresolved questions and some parallelism', () => {
    const verdict = classifyTriage({
      ...trivialInput,
      size: 'medium',
      kind: 'maintenance',
      clarity: 'needs-questions',
      parallelism: 'some',
    })

    expect(verdict.stageTiers.grill).toBe('sonnet')
    expect(verdict.stageTiers.survey).toBe('sonnet')
    expect(verdict.stageTiers.consolidate).toBe('opus')
    expect(verdict.stageTiers.tasking).toBe('sonnet')
  })

  it('reports matrix configuration errors when no fallback or profile exists', () => {
    expect(() => classifyTriage(trivialInput, { ...routingMatrix, routeRules: [] })).toThrow('no fallback rule')
    expect(() =>
      classifyTriage(trivialInput, { ...routingMatrix, routeProfiles: routingMatrix.routeProfiles.slice(1) }),
    ).toThrow('no profile')
  })

  it('matches empty and constrained rule conditions', () => {
    expect(matchesCondition(trivialInput, {})).toBe(true)
    expect(matchesCondition(trivialInput, { size: ['large'] })).toBe(false)
  })
})
