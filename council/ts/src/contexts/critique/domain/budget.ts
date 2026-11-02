import { JsonFields } from './json.js'
import { DEFAULT_SEARCH_LIMITS, readSearchLimits, type SearchLimits } from './packet-search.js'
import { CRITIQUE_STAGES, type CritiqueStage, type Round } from './stage.js'

/**
 * Antigravity reports no quota headless, so the runner counts its own agy
 * launches per day. Past the reserve threshold only the reserved stages may
 * still use agy; at the daily cap none may.
 */
export interface AntigravityQuota {
  readonly daily_cap: number
  readonly reserve_threshold: number
  readonly reserved_stages: readonly CritiqueStage[]
}

/**
 * What routing caps beyond each critic's own price cap and time limit, under
 * the keys routing gives them.
 */
export interface CritiqueLimits {
  /** The most the Vibe critics of one flow may spend, across all its runs. */
  readonly budget: { readonly vibe_flow_cap_usd: number }
  readonly antigravity_quota: AntigravityQuota
  /** How much one answer of the runner's search server may hold (ADR-0006). */
  readonly search: SearchLimits
}

/** #167's limits, for a routing file that does not state its own. */
export const DEFAULT_LIMITS: CritiqueLimits = {
  antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec', 'feature-review'] },
  budget: { vibe_flow_cap_usd: 5 },
  search: DEFAULT_SEARCH_LIMITS,
}

/** Reads the limits from routing or a run manifest; a missing key keeps #167's value. */
export function readLimits(file: JsonFields): CritiqueLimits {
  const quota = file.has('antigravity_quota') ? file.object('antigravity_quota') : undefined
  return {
    antigravity_quota:
      quota === undefined
        ? DEFAULT_LIMITS.antigravity_quota
        : {
            daily_cap: quota.number('daily_cap'),
            reserve_threshold: quota.number('reserve_threshold'),
            reserved_stages: quota.oneOfList('reserved_stages', CRITIQUE_STAGES),
          },
    budget: file.has('budget') ? { vibe_flow_cap_usd: file.object('budget').number('vibe_flow_cap_usd') } : DEFAULT_LIMITS.budget,
    search: file.has('search') ? readSearchLimits(file.object('search')) : DEFAULT_LIMITS.search,
  }
}

/**
 * One critic launch, appended to the ledger as soon as it ends, so the spend
 * and the agy count stay true even for a run that is never recorded.
 * Metadata only, like every ledger line.
 */
export interface InvocationLine {
  readonly type: 'invocation'
  readonly recorded_at: string
  readonly repository: string
  readonly flow: string
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly round: Round
  readonly critic_id: string
  readonly client: string
  readonly kind: 'vibe' | 'agy'
  readonly family: string
  readonly model: string
  readonly status: string
  /** What the launch counts against the flow's Vibe cap; always 0 for agy. */
  readonly charged_usd: number
}

/**
 * What the budget reads back from one invocation line, and the run it was
 * launched for; setup's checks launch for no run.
 */
export type LoggedInvocation = Pick<InvocationLine, 'recorded_at' | 'repository' | 'flow' | 'kind' | 'charged_usd'> & {
  readonly run_id: string | undefined
}

/**
 * The invocation lines of a ledger. Any other line, and a line that is not
 * a JSON object of the right shape (a write cut short, say), is skipped.
 */
export function invocationLines(ledger: string): readonly LoggedInvocation[] {
  return ledger.split('\n').flatMap((text) => {
    try {
      const line = JsonFields.parse(text)
      if (!line.has('type') || line.string('type') !== 'invocation') return []
      return [
        {
          charged_usd: line.number('charged_usd'),
          flow: line.string('flow'),
          kind: line.oneOf('kind', ['vibe', 'agy']),
          recorded_at: line.string('recorded_at'),
          repository: line.string('repository'),
          run_id: line.optionalString('run_id'),
        },
      ]
    } catch {
      return []
    }
  })
}

/** Everything the Vibe critics of one flow of one repository were charged. */
export function flowSpend(lines: readonly LoggedInvocation[], repository: string, flow: string): number {
  return lines
    .filter((line) => line.kind === 'vibe' && line.repository === repository && line.flow === flow)
    .reduce((total, line) => total + line.charged_usd, 0)
}

/** A feature's Vibe spend under one of its flow keys. */
export interface FlowSpend {
  readonly flow: string
  readonly spent_usd: number
}

/** What each of a feature's flow keys was charged, in the order of `flows`: the spec's flow, then its draft's. */
export function spendByFlow(lines: readonly LoggedInvocation[], repository: string, flows: readonly string[]): readonly FlowSpend[] {
  return flows.map((flow) => ({ flow, spent_usd: flowSpend(lines, repository, flow) }))
}

export function totalSpend(spends: readonly FlowSpend[]): number {
  return spends.reduce((total, spend) => total + spend.spent_usd, 0)
}

/** The UTC day of `now`, as `YYYY-MM-DD`: the day the Antigravity quota counts. */
export function utcDay(now: Date): string {
  return now.toISOString().slice(0, 10)
}

/** agy launches on this machine on the UTC day of `now`. */
export function agyInvocationsOn(lines: readonly LoggedInvocation[], now: Date): number {
  const day = utcDay(now)
  return lines.filter((line) => line.kind === 'agy' && line.recorded_at.slice(0, 10) === day).length
}

/** Why the flow may launch no more Vibe critics, or undefined while it may. */
export function budgetReached(spend: number, limits: CritiqueLimits): string | undefined {
  const cap = limits.budget.vibe_flow_cap_usd
  return spend >= cap ? `the flow's Vibe spend $${spend.toFixed(4)} has reached its $${cap.toFixed(2)} cap` : undefined
}

/** Why this stage may launch no more agy critics today, or undefined while it may. */
export function quotaReached(count: number, quota: AntigravityQuota, stage: CritiqueStage): string | undefined {
  if (count >= quota.daily_cap) {
    return `antigravity: ${String(count)} invocations today have reached the daily cap of ${String(quota.daily_cap)}`
  }
  if (count >= quota.reserve_threshold && !quota.reserved_stages.includes(stage)) {
    return `antigravity: ${String(count)} invocations today have reached the reserve threshold of ${String(quota.reserve_threshold)}; the rest is kept for ${quota.reserved_stages.join(' and ')}`
  }
  return undefined
}
