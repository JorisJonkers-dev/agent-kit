import { describe, expect, it } from 'vitest'

import { buildTriageGatePayload, triageLensProfile, type TriageGateRecommendation } from './gate.js'
import { classifyTriage, recommendLenses, type TriageInput } from './index.js'

const baseInput: TriageInput = {
  clarity: 'clear',
  kind: 'feature',
  landscape: 'brownfield',
  parallelism: 'none',
  risk: 'medium',
  size: 'medium',
}

function payloadFor(input: TriageInput, recommendation: TriageGateRecommendation = recommendLenses(triageLensProfile(input))) {
  return buildTriageGatePayload({
    input,
    recommendation,
    verdict: classifyTriage(input),
  })
}

describe('triage gate payload', () => {
  it('maps a direct triage classification and recommendation into a single-topology payload', () => {
    const input: TriageInput = {
      ...baseInput,
      clarity: 'clear',
      kind: 'ui-tweak',
      parallelism: 'none',
      risk: 'low',
      size: 'trivial',
    }
    const payload = payloadFor(input)

    expect(payload).toMatchObject({
      schema_version: 1,
      input,
      route: 'direct',
      matched_rule_id: 'trivial-clear-direct',
      council_worthy: false,
      topology: 'single',
      parallelism_score: 10,
      shared_file_risk: 'low',
      verification_score: 20,
      budget_estimate: {
        worker_count: 3,
        rounds: 1,
        estimated_model_calls: 5,
        tier: 'medium',
      },
      classification: {
        candidate_routes: ['direct', 'delta'],
        dag_shape: 'single-minimal-task',
        stage_tiers: classifyTriage(input).stageTiers,
      },
      lens_recommendation: {
        recommended_lenses: payload.lens_recommendation.recommended_lenses,
        considered_count: payload.lens_recommendation.considered_count,
        dropped_count: payload.lens_recommendation.dropped_count,
        rationale: payload.lens_recommendation.rationale,
      },
      plan: {
        direct_worker_policy: 'never-during-plan',
        executes_workers: false,
      },
    })
    expect(payload.reasons).toContain('A trivial, clear, low-risk change should not fan out planning overhead.')
    expect(payload.lens_recommendation.recommended_lenses.length).toBeGreaterThan(0)
  })

  it('maps delta, full, and program routes to deterministic sequential, hybrid, and parallel topologies', () => {
    expect(payloadFor({ ...baseInput, kind: 'refactor', parallelism: 'none' })).toMatchObject({
      council_worthy: true,
      route: 'delta',
      topology: 'sequential',
    })
    expect(
      payloadFor({
        ...baseInput,
        kind: 'api',
        parallelism: 'some',
      }),
    ).toMatchObject({
      council_worthy: true,
      route: 'full',
      topology: 'hybrid',
    })
    expect(
      payloadFor({
        ...baseInput,
        kind: 'feature',
        parallelism: 'high',
      }),
    ).toMatchObject({
      council_worthy: true,
      route: 'program',
      topology: 'parallel',
    })
    expect(payloadFor({ ...baseInput, parallelism: 'none', size: 'program' })).toMatchObject({
      route: 'program',
      topology: 'hybrid',
    })
  })

  it('derives bounded scores and a large budget from classification and lens recommendation depth', () => {
    const recommendation = recommendLenses(
      triageLensProfile({
        ...baseInput,
        clarity: 'unclear',
        parallelism: 'high',
        risk: 'critical',
        size: 'program',
      }),
    )
    const payload = payloadFor(
      {
        ...baseInput,
        clarity: 'unclear',
        parallelism: 'high',
        risk: 'critical',
        size: 'program',
      },
      recommendation,
    )

    expect(payload.parallelism_score).toBe(100)
    expect(payload.shared_file_risk).toBe('high')
    expect(payload.verification_score).toBe(100)
    expect(payload.budget_estimate).toEqual({
      estimated_model_calls: recommendation.workerCount * recommendation.rounds + 3,
      rounds: recommendation.rounds,
      tier: 'program',
      worker_count: recommendation.workerCount,
    })
    expect(payload.lens_recommendation.considered_count).toBe(recommendation.negotiation.considered.length)
    expect(payload.lens_recommendation.dropped_count).toBe(recommendation.negotiation.dropped.length)
  })

  it('keeps scores deterministic for custom recommendation edges', () => {
    const lowPayload = payloadFor(
      {
        ...baseInput,
        kind: 'maintenance',
        parallelism: 'some',
        risk: 'medium',
      },
      {
        lenses: [],
        negotiation: { considered: [], dropped: [], tensionsBalanced: [] },
        rationale: ['custom empty recommendation'],
        rounds: 1,
        workerCount: 1,
      },
    )
    const mediumPayload = payloadFor(
      {
        ...baseInput,
        kind: 'maintenance',
        parallelism: 'some',
        risk: 'high',
      },
      {
        lenses: [
          {
            category: 'test',
            focus: 'verify touched paths',
            id: 'verification',
            name: 'Verification',
            score: 10,
            suggestedTier: 'codex-medium',
          },
          {
            category: 'refactor',
            focus: 'split shared edits',
            id: 'shared-files',
            name: 'Shared files',
            score: 9,
            suggestedTier: 'codex-high',
          },
        ],
        negotiation: { considered: [], dropped: [], tensionsBalanced: [] },
        rationale: ['custom multi-worker recommendation'],
        rounds: 2,
        workerCount: 2,
      },
    )

    expect(lowPayload).toMatchObject({
      parallelism_score: 50,
      shared_file_risk: 'medium',
      verification_score: 55,
    })
    expect(mediumPayload).toMatchObject({
      parallelism_score: 55,
      shared_file_risk: 'high',
      verification_score: 90,
    })
  })

  it('builds a lens profile from triage dimensions with optional signals', () => {
    expect(triageLensProfile(baseInput, ['shared files', ' '])).toEqual({
      clarity: 'clear',
      kind: 'feature',
      landscape: 'brownfield',
      parallelism: 'none',
      risk: 'medium',
      signals: ['shared files'],
      size: 'medium',
    })
  })
})
