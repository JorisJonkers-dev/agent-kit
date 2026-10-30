import type { ModelFamily } from './data-policy.js'

/** One CLI pinned to one model. Model names live here and in the critic agent files only. */
export interface VibeCriticClient {
  readonly kind: 'vibe'
  readonly binary: string
  /** The read-only `critic-<model>` Vibe agent that pins the model and its prices. */
  readonly agent: string
  readonly model: string
  readonly family: ModelFamily
}

export type CriticClient = VibeCriticClient

export interface RosterSlot {
  readonly client: string
  readonly role: string
  readonly lenses: readonly string[]
  readonly max_price_usd: number
  readonly time_limit_s: number
}

export interface StageRouting {
  readonly distinct_families: boolean
  readonly base: readonly RosterSlot[]
}

export interface Routing {
  readonly version: 1
  readonly clients: Readonly<Record<string, CriticClient>>
  readonly stages: Readonly<Record<string, StageRouting>>
}

/** A critic resolved for one run: the slot plus the client it runs through. */
export interface RosterCritic extends RosterSlot {
  readonly critic_id: string
  readonly client_config: CriticClient
}

export interface DroppedCritic {
  readonly client: string
  readonly family: ModelFamily
  readonly reason: 'family-not-allowed' | 'orchestrator-family'
}

export interface Roster {
  readonly depth: 'base'
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

export function parseRouting(text: string): Routing {
  return JSON.parse(text) as Routing
}

export interface RosterRequest {
  readonly stage: string
  readonly allowedFamilies: readonly ModelFamily[]
  readonly orchestratorFamily: string
}

/**
 * Resolves the base roster of a stage. A critic is dropped when the
 * repository's allowlist does not permit its family, or when it shares the
 * orchestrator's family, which adjudicates its findings.
 */
export function resolveRoster(routing: Routing, request: RosterRequest): Roster {
  const stage = routing.stages[request.stage]
  if (stage === undefined) throw new Error(`routing has no stage ${request.stage}`)
  const critics: RosterCritic[] = []
  const dropped: DroppedCritic[] = []
  for (const slot of stage.base) {
    const client = routing.clients[slot.client]
    if (client === undefined) throw new Error(`routing stage ${request.stage} names unknown client ${slot.client}`)
    const reason = dropReason(client.family, request)
    if (reason === undefined) {
      critics.push({ ...slot, client_config: client, critic_id: `c${String(critics.length + 1)}-${slot.client}` })
    } else {
      dropped.push({ client: slot.client, family: client.family, reason })
    }
  }
  return { critics, depth: 'base', dropped }
}

function dropReason(family: ModelFamily, request: RosterRequest): DroppedCritic['reason'] | undefined {
  if (!request.allowedFamilies.includes(family)) return 'family-not-allowed'
  if (family === request.orchestratorFamily) return 'orchestrator-family'
  return undefined
}
