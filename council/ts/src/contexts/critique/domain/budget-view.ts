import { quotaReached, type FlowSpend } from './budget.js'
import type { Routing, RosterSlot } from './routing.js'
import { CRITIQUE_STAGES, type CritiqueStage } from './stage.js'
import { STAGE_TABLE } from './stage-table.js'

/** A routed Antigravity slot the quota holds, and the client prepare would try in its place. */
export interface SlotFallback {
  readonly client: string
  /** The first of the slot's fallbacks that is not Antigravity, in routing order; null leaves the slot empty. */
  readonly to: string | null
  /** True for a slot of the deep roster. */
  readonly deep?: true
}

/** What the limits would do to one stage if it ran now. */
export interface StageOutlook {
  readonly stage: CritiqueStage
  /**
   * The flow key the stage's run would charge: the draft's for decisions,
   * the spec's for every other critique stage; null when the feature has
   * none yet, and always for a review stage, so the run would start a flow
   * of its own, keyed by its run id.
   */
  readonly flow: string | null
  /**
   * Why a Vibe critic of the stage, a fallback included, would be skipped
   * now: the gateway's reason Vibe cannot start, at 100% of one of its plan
   * windows; null while it may launch.
   */
  readonly vibe: string | null
  /** Why the stage's routed Antigravity critics would not launch now, as prepare words it; null when none is held. */
  readonly antigravity: string | null
  readonly falls_back: readonly SlotFallback[]
}

/**
 * What one feature has drawn on: the spend under its spec's and its draft's
 * flow keys, today's agy launches, and why Vibe cannot start at all.
 */
export interface FeatureDraw {
  readonly spec: FlowSpend | null
  readonly draft: FlowSpend | null
  readonly agyToday: number
  /** The gateway's reason Vibe cannot start, at 100% of a plan window; null while it can. */
  readonly plan: string | null
}

/**
 * Each stage routing defines, in flow order, with what Vibe's plan limit and
 * today's Antigravity count would do to it. The plan-limit and quota rules
 * are fan-out's and prepare's own; the fallback named is the routed one,
 * before the repository's allowlist, the orchestrator's family and this
 * machine's readiness, which prepare checks too, have had a say.
 */
export function stageOutlooks(routing: Routing, draw: FeatureDraw): readonly StageOutlook[] {
  return CRITIQUE_STAGES.flatMap((stage) => {
    const charged = { draft: draw.draft, run: null, spec: draw.spec }[STAGE_TABLE[stage].flowKey]
    const stageRoutes = routing.stages[stage]
    if (stageRoutes === undefined) return []
    const held: readonly { readonly slot: RosterSlot; readonly deep?: true }[] = [
      ...stageRoutes.base.map((slot) => ({ slot })),
      ...stageRoutes.deep.map((slot) => ({ deep: true as const, slot })),
    ].filter(({ slot }) => routing.clients[slot.client]?.kind === 'agy')
    const quota = held.length === 0 ? undefined : quotaReached(draw.agyToday, routing.limits.antigravity_quota, stage)
    return [
      {
        antigravity: quota ?? null,
        falls_back: quota === undefined ? [] : held.map(({ slot, deep }) => slotFallback(routing, slot, deep)),
        flow: charged?.flow ?? null,
        stage,
        vibe: draw.plan,
      },
    ]
  })
}

function slotFallback(routing: Routing, slot: RosterSlot, deep: true | undefined): SlotFallback {
  const to = (routing.fallbacks[slot.client] ?? []).find((name) => routing.clients[name]?.kind !== 'agy')
  return { client: slot.client, to: to ?? null, ...(deep === undefined ? {} : { deep }) }
}
