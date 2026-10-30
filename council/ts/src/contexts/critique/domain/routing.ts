import type { CriticIdentity } from './critic.js'
import { CritiqueError } from './outcomes.js'
import type { ModelFamily, OrchestratorFamily } from './family.js'
import type { CritiqueStage, Depth } from './stage.js'

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
  readonly stages: Readonly<Partial<Record<CritiqueStage, StageRouting>>>
}

/** A critic resolved for one run: the slot plus the client it runs through. */
export interface RosterCritic extends RosterSlot {
  readonly critic_id: string
  readonly client_config: CriticClient
}

export const DROP_REASONS = ['family-not-allowed', 'orchestrator-family', 'same-family'] as const

export interface DroppedCritic {
  readonly client: string
  readonly family: ModelFamily
  readonly reason: (typeof DROP_REASONS)[number]
}

export interface Roster {
  readonly depth: Depth
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

export function parseRouting(text: string): Routing {
  return JSON.parse(text) as Routing
}

export interface RosterRequest {
  readonly stage: CritiqueStage
  readonly allowedFamilies: readonly ModelFamily[]
  readonly orchestratorFamily: OrchestratorFamily
}

/**
 * Resolves the base roster of a stage. A critic is dropped when the
 * repository's allowlist does not permit its family, when it shares the
 * orchestrator's family, which adjudicates its findings, or when the stage
 * requires distinct families and an earlier critic already has its family.
 */
export function resolveRoster(routing: Routing, request: RosterRequest): Roster {
  const stage = routing.stages[request.stage]
  if (stage === undefined) throw new CritiqueError('invalid-routing', `routing has no stage ${request.stage}`)
  const critics: RosterCritic[] = []
  const dropped: DroppedCritic[] = []
  for (const slot of stage.base) {
    const client = routing.clients[slot.client]
    if (client === undefined) {
      throw new CritiqueError('invalid-routing', `routing stage ${request.stage} names unknown client ${slot.client}`)
    }
    const reason =
      dropReason(client.family, request) ??
      (stage.distinct_families && critics.some((critic) => critic.client_config.family === client.family)
        ? 'same-family'
        : undefined)
    if (reason === undefined) {
      critics.push({ ...slot, client_config: client, critic_id: `c${String(critics.length + 1)}-${slot.client}` })
    } else {
      dropped.push({ client: slot.client, family: client.family, reason })
    }
  }
  return { critics, depth: 'base', dropped }
}

/** The identity a roster critic carries into fan-out, record and the ledger. */
export function criticIdentity(critic: RosterCritic): CriticIdentity {
  return {
    client: critic.client,
    critic_id: critic.critic_id,
    family: critic.client_config.family,
    lenses: critic.lenses,
    model: critic.client_config.model,
    role: critic.role,
  }
}

function dropReason(family: ModelFamily, request: RosterRequest): DroppedCritic['reason'] | undefined {
  if (!request.allowedFamilies.includes(family)) return 'family-not-allowed'
  if (family === request.orchestratorFamily) return 'orchestrator-family'
  return undefined
}
