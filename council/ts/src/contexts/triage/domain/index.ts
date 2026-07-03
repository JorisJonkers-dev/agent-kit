import matrixJson from './routing-matrix.json' with { type: 'json' }

export type TriageRoute = 'direct' | 'delta' | 'full' | 'program'
export type TriageSize = 'trivial' | 'small' | 'medium' | 'large' | 'program'
export type TriageLandscape = 'brownfield' | 'greenfield'
export type TriageKind =
  | 'ui-tweak'
  | 'bugfix'
  | 'hotfix'
  | 'refactor'
  | 'api'
  | 'feature'
  | 'cross-cutting'
  | 'maintenance'
  | 'design-system'
  | 'prototype'
export type TriageRisk = 'low' | 'medium' | 'high' | 'critical'
export type TriageClarity = 'clear' | 'needs-questions' | 'unclear'
export type TriageParallelism = 'none' | 'some' | 'high'
export type TriageStage =
  | 'grill'
  | 'survey'
  | 'plan'
  | 'critique'
  | 'consolidate'
  | 'tasking'
  | 'verify'
export type TriageTier = 'skip' | 'haiku' | 'sonnet' | 'opus'
export type TriageDagShape =
  | 'single-minimal-task'
  | 'delta-task-dag'
  | 'full-task-dag'
  | 'parallel-program-dag'

export interface TriageInput {
  readonly size: TriageSize
  readonly landscape: TriageLandscape
  readonly kind: TriageKind
  readonly risk: TriageRisk
  readonly clarity: TriageClarity
  readonly parallelism: TriageParallelism
}

export type TriageCondition = Partial<{
  readonly size: readonly TriageSize[]
  readonly landscape: readonly TriageLandscape[]
  readonly kind: readonly TriageKind[]
  readonly risk: readonly TriageRisk[]
  readonly clarity: readonly TriageClarity[]
  readonly parallelism: readonly TriageParallelism[]
}>

export type StageTiers = Readonly<Record<TriageStage, TriageTier>>

export interface RoutingMatrix {
  readonly $schema: './routing-matrix.schema.json'
  readonly schemaVersion: 1
  readonly source: {
    readonly repository: string
    readonly path: string
    readonly ref: string
    readonly transcribedAt: string
    readonly notes: readonly string[]
  }
  readonly dimensions: Readonly<Record<keyof TriageInput, readonly string[]>>
  readonly useCaseScores: readonly UseCaseScore[]
  readonly routeProfiles: readonly RouteProfile[]
  readonly routeRules: readonly RouteRule[]
  readonly stageAdjustments: readonly StageAdjustment[]
}

export interface UseCaseScore {
  readonly id: string
  readonly category: string
  readonly best: readonly string[]
  readonly avoid: readonly string[]
  readonly scores: Readonly<Record<string, number>>
}

export interface RouteProfile {
  readonly route: TriageRoute
  readonly basis: string
  readonly dagShape: TriageDagShape
  readonly planExecutesWorkers: false
  readonly stageTiers: StageTiers
}

export interface RouteRule {
  readonly id: string
  readonly route: TriageRoute
  readonly reason: string
  readonly useCaseRefs: readonly string[]
  readonly when: TriageCondition
}

export interface StageAdjustment {
  readonly id: string
  readonly reason: string
  readonly when: TriageCondition
  readonly minTiers: Partial<StageTiers>
}

export interface TriageVerdict {
  readonly route: TriageRoute
  readonly matchedRuleId: string
  readonly candidateRoutes: readonly TriageRoute[]
  readonly reasons: readonly string[]
  readonly useCaseRefs: readonly string[]
  readonly stageTiers: StageTiers
  readonly plan: {
    readonly dagShape: TriageDagShape
    readonly executesWorkers: false
    readonly directWorkerPolicy: 'never-during-plan'
  }
}

const tierRank: Readonly<Record<TriageTier, number>> = {
  skip: 0,
  haiku: 1,
  sonnet: 2,
  opus: 3,
}

export function loadRoutingMatrix(value: unknown = matrixJson): RoutingMatrix {
  return parseRoutingMatrix(value)
}

export function parseRoutingMatrix(value: unknown): RoutingMatrix {
  if (!isRecord(value) || value.$schema !== './routing-matrix.schema.json' || value.schemaVersion !== 1) {
    throw new TypeError('Routing matrix must declare schema version 1')
  }
  if (!Array.isArray(value.routeProfiles) || !Array.isArray(value.routeRules) || !Array.isArray(value.stageAdjustments)) {
    throw new TypeError('Routing matrix must include profiles, rules, and stage adjustments')
  }
  return value as unknown as RoutingMatrix
}

export const routingMatrix = loadRoutingMatrix()

export function classifyTriage(input: TriageInput, matrix: RoutingMatrix = routingMatrix): TriageVerdict {
  const matches = matrix.routeRules.filter((rule) => matchesCondition(input, rule.when))
  const selectedRule = matches[0]
  if (selectedRule === undefined) {
    throw new Error('Routing matrix has no fallback rule')
  }
  const profile = matrix.routeProfiles.find((routeProfile) => routeProfile.route === selectedRule.route)
  if (profile === undefined) {
    throw new Error(`Routing matrix has no profile for ${selectedRule.route}`)
  }
  const candidateRules = matches.length > 1 ? matches.filter((rule) => hasSpecificCondition(rule.when)) : matches
  const appliedAdjustments = matrix.stageAdjustments.filter((adjustment) => matchesCondition(input, adjustment.when))
  return {
    route: selectedRule.route,
    matchedRuleId: selectedRule.id,
    candidateRoutes: uniqueRoutes(candidateRules.map((rule) => rule.route)),
    reasons: [selectedRule.reason, ...appliedAdjustments.map((adjustment) => adjustment.reason)],
    useCaseRefs: selectedRule.useCaseRefs,
    stageTiers: applyStageAdjustments(profile.stageTiers, appliedAdjustments),
    plan: {
      dagShape: profile.dagShape,
      executesWorkers: profile.planExecutesWorkers,
      directWorkerPolicy: 'never-during-plan',
    },
  }
}

export function matchesCondition(input: TriageInput, condition: TriageCondition): boolean {
  return (
    includesOrAny(condition.size, input.size) &&
    includesOrAny(condition.landscape, input.landscape) &&
    includesOrAny(condition.kind, input.kind) &&
    includesOrAny(condition.risk, input.risk) &&
    includesOrAny(condition.clarity, input.clarity) &&
    includesOrAny(condition.parallelism, input.parallelism)
  )
}

function applyStageAdjustments(base: StageTiers, adjustments: readonly StageAdjustment[]): StageTiers {
  return adjustments.reduce<StageTiers>(
    (stageTiers, adjustment) => mergeMinTiers(stageTiers, adjustment.minTiers),
    { ...base },
  )
}

function mergeMinTiers(stageTiers: StageTiers, minTiers: Partial<StageTiers>): StageTiers {
  return {
    grill: maxTier(stageTiers.grill, minTiers.grill),
    survey: maxTier(stageTiers.survey, minTiers.survey),
    plan: maxTier(stageTiers.plan, minTiers.plan),
    critique: maxTier(stageTiers.critique, minTiers.critique),
    consolidate: maxTier(stageTiers.consolidate, minTiers.consolidate),
    tasking: maxTier(stageTiers.tasking, minTiers.tasking),
    verify: maxTier(stageTiers.verify, minTiers.verify),
  }
}

function maxTier(current: TriageTier, minimum: TriageTier | undefined): TriageTier {
  return minimum === undefined || tierRank[current] >= tierRank[minimum] ? current : minimum
}

function includesOrAny<T extends string>(allowed: readonly T[] | undefined, value: T): boolean {
  return allowed === undefined || allowed.includes(value)
}

function hasSpecificCondition(condition: TriageCondition): boolean {
  return Object.keys(condition).length > 0
}

function uniqueRoutes(routes: readonly TriageRoute[]): readonly TriageRoute[] {
  return [...new Set(routes)]
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
