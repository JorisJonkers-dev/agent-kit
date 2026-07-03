import type { LensProblemProfile, LensRecommendation, LensRef } from './recommendation.js'

export type TriageGateTopology = 'single' | 'sequential' | 'parallel' | 'hybrid'
export type TriageGateSharedFileRisk = 'low' | 'medium' | 'high'
export type TriageGateRoute = 'direct' | 'delta' | 'full' | 'program'
export type TriageGateSize = 'trivial' | 'small' | 'medium' | 'large' | 'program'
export type TriageGateLandscape = 'brownfield' | 'greenfield'
export type TriageGateRisk = 'low' | 'medium' | 'high' | 'critical'
export type TriageGateClarity = 'clear' | 'needs-questions' | 'unclear'
export type TriageGateParallelism = 'none' | 'some' | 'high'
export type TriageGateDagShape =
  | 'single-minimal-task'
  | 'delta-task-dag'
  | 'full-task-dag'
  | 'parallel-program-dag'

export interface TriageGateInput {
  readonly size: TriageGateSize
  readonly landscape: TriageGateLandscape
  readonly kind: string
  readonly risk: TriageGateRisk
  readonly clarity: TriageGateClarity
  readonly parallelism: TriageGateParallelism
}

export interface TriageGateVerdict {
  readonly route: TriageGateRoute
  readonly matchedRuleId: string
  readonly candidateRoutes: readonly TriageGateRoute[]
  readonly reasons: readonly string[]
  readonly useCaseRefs: readonly string[]
  readonly stageTiers: Readonly<Record<string, string>>
  readonly plan: {
    readonly dagShape: TriageGateDagShape
    readonly executesWorkers: false
    readonly directWorkerPolicy: 'never-during-plan'
  }
}

export type TriageGateRecommendation = LensRecommendation

export interface TriageGatePayload {
  readonly schema_version: 1
  readonly input: TriageGateInput
  readonly route: TriageGateRoute
  readonly matched_rule_id: string
  readonly council_worthy: boolean
  readonly topology: TriageGateTopology
  readonly parallelism_score: number
  readonly shared_file_risk: TriageGateSharedFileRisk
  readonly verification_score: number
  readonly budget_estimate: {
    readonly worker_count: number
    readonly rounds: number
    readonly estimated_model_calls: number
    readonly tier: 'small' | 'medium' | 'large' | 'program'
  }
  readonly classification: {
    readonly candidate_routes: readonly TriageGateRoute[]
    readonly use_case_refs: readonly string[]
    readonly dag_shape: TriageGateDagShape
    readonly stage_tiers: Readonly<Record<string, string>>
  }
  readonly lens_recommendation: {
    readonly recommended_lenses: readonly LensRef[]
    readonly considered_count: number
    readonly dropped_count: number
    readonly rationale: readonly string[]
  }
  readonly plan: {
    readonly executes_workers: false
    readonly direct_worker_policy: 'never-during-plan'
  }
  readonly reasons: readonly string[]
}

export interface BuildTriageGatePayloadInput {
  readonly input: TriageGateInput
  readonly verdict: TriageGateVerdict
  readonly recommendation: TriageGateRecommendation
}

export function triageLensProfile(input: TriageGateInput, signals: readonly string[] = []): LensProblemProfile {
  return {
    clarity: input.clarity,
    kind: input.kind,
    landscape: input.landscape,
    parallelism: input.parallelism,
    risk: input.risk,
    signals: signals.filter((signal) => signal.trim().length > 0),
    size: input.size,
  }
}

export function buildTriageGatePayload({
  input,
  recommendation,
  verdict,
}: BuildTriageGatePayloadInput): TriageGatePayload {
  const topology = deriveTopology(input, verdict)
  const councilWorthy = verdict.route !== 'direct'
  const budgetEstimate = deriveBudgetEstimate(verdict.route, recommendation)
  return {
    schema_version: 1,
    budget_estimate: budgetEstimate,
    classification: {
      candidate_routes: verdict.candidateRoutes,
      dag_shape: verdict.plan.dagShape,
      stage_tiers: verdict.stageTiers,
      use_case_refs: verdict.useCaseRefs,
    },
    council_worthy: councilWorthy,
    input,
    lens_recommendation: {
      considered_count: recommendation.negotiation.considered.length,
      dropped_count: recommendation.negotiation.dropped.length,
      rationale: recommendation.rationale,
      recommended_lenses: recommendation.lenses,
    },
    matched_rule_id: verdict.matchedRuleId,
    parallelism_score: deriveParallelismScore(input, topology, recommendation),
    plan: {
      direct_worker_policy: verdict.plan.directWorkerPolicy,
      executes_workers: verdict.plan.executesWorkers,
    },
    reasons: verdict.reasons,
    route: verdict.route,
    shared_file_risk: deriveSharedFileRisk(input, topology, verdict.route, recommendation),
    topology,
    verification_score: deriveVerificationScore(input, verdict.route, recommendation),
  }
}

function deriveTopology(input: TriageGateInput, verdict: TriageGateVerdict): TriageGateTopology {
  if (verdict.plan.dagShape === 'single-minimal-task') return 'single'
  if (verdict.plan.dagShape === 'parallel-program-dag') return input.parallelism === 'high' ? 'parallel' : 'hybrid'
  if (verdict.plan.dagShape === 'delta-task-dag') return input.parallelism === 'none' ? 'sequential' : 'hybrid'
  return input.parallelism === 'none' ? 'sequential' : 'hybrid'
}

function deriveParallelismScore(
  input: TriageGateInput,
  topology: TriageGateTopology,
  recommendation: TriageGateRecommendation,
): number {
  const base = input.parallelism === 'high' ? 80 : input.parallelism === 'some' ? 45 : 0
  const topologyBonus = topology === 'parallel' ? 10 : topology === 'hybrid' ? 5 : 0
  const workerBonus = Math.min((recommendation.workerCount - 1) * 5, 10)
  return cappedScore(base + topologyBonus + workerBonus)
}

function deriveSharedFileRisk(
  input: TriageGateInput,
  topology: TriageGateTopology,
  route: TriageGateRoute,
  recommendation: TriageGateRecommendation,
): TriageGateSharedFileRisk {
  const topologyPressure = { hybrid: 45, parallel: 55, sequential: 25, single: 0 }[topology]
  const routePressure = { delta: 5, direct: 0, full: 10, program: 20 }[route]
  const riskPressure = input.risk === 'critical' ? 25 : input.risk === 'high' ? 15 : 0
  const workerPressure = recommendation.workerCount > 1 ? 10 : 0
  const score = topologyPressure + routePressure + riskPressure + workerPressure
  if (score >= 70) return 'high'
  return score >= 35 ? 'medium' : 'low'
}

function deriveVerificationScore(
  input: TriageGateInput,
  route: TriageGateRoute,
  recommendation: TriageGateRecommendation,
): number {
  const base = { critical: 90, high: 70, low: 20, medium: 45 }[input.risk]
  const routePressure = { delta: 10, direct: 0, full: 15, program: 10 }[route]
  const clarityPressure = input.clarity === 'clear' ? 0 : 10
  const roundPressure = recommendation.rounds > 1 ? 10 : 0
  return cappedScore(base + routePressure + clarityPressure + roundPressure)
}

function deriveBudgetEstimate(
  route: TriageGateRoute,
  recommendation: TriageGateRecommendation,
): TriageGatePayload['budget_estimate'] {
  const estimatedModelCalls = recommendation.workerCount * recommendation.rounds + (route === 'direct' ? 2 : 3)
  return {
    estimated_model_calls: estimatedModelCalls,
    rounds: recommendation.rounds,
    tier: budgetTier(route, estimatedModelCalls, recommendation.workerCount),
    worker_count: recommendation.workerCount,
  }
}

function budgetTier(
  route: TriageGateRoute,
  estimatedModelCalls: number,
  workerCount: number,
): TriageGatePayload['budget_estimate']['tier'] {
  if (route === 'program' || workerCount >= 6) return 'program'
  if (estimatedModelCalls >= 10) return 'large'
  return estimatedModelCalls >= 5 ? 'medium' : 'small'
}

function cappedScore(score: number): number {
  return Math.max(0, Math.min(score, 100))
}
