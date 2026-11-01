import { NO_DEPTH_SIGNALS, readDepthSignalsConfig, type DepthSignalsConfig } from './depth.js'
import { MODEL_FAMILIES, type ModelFamily, type OrchestratorFamily } from './family.js'
import { JsonFields } from './json.js'
import { CritiqueError } from './outcomes.js'
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

/**
 * Antigravity in print mode. agy also offers Claude and GPT-OSS models; those
 * are never critics, so the model must be a Gemini one and the family Google.
 */
export interface AgyCriticClient {
  readonly kind: 'agy'
  readonly binary: string
  readonly model: string
  readonly family: 'google'
}

export type CriticClient = VibeCriticClient | AgyCriticClient

export const CLIENT_KINDS = ['vibe', 'agy'] as const satisfies readonly CriticClient['kind'][]

/** Whether a critic client can run on this machine now; `detail` says why not, in setup's words where setup has them. */
export type ClientReadiness = { readonly ready: true } | { readonly ready: false; readonly detail: string }

export interface RosterSlot {
  readonly client: string
  readonly role: string
  readonly lenses: readonly string[]
  /** Vibe's price cap. Antigravity is quota, not money, so agy ignores it. */
  readonly max_price_usd: number
  readonly time_limit_s: number
}

export interface StageRouting {
  readonly distinct_families: boolean
  readonly base: readonly RosterSlot[]
  /** The critics deep depth adds after the base roster, each subject to the same family rules. */
  readonly deep: readonly RosterSlot[]
  /** What makes the stage go deep when the caller names no depth. */
  readonly depth_signals: DepthSignalsConfig
}

/** When critique stats may draw a conclusion. They live in routing, beside what they tune; stats never writes them. */
export interface StatsThresholds {
  /** A cell with fewer accepted plus rejected findings than this gets no routing suggestion. */
  readonly min_adjudicated_findings: number
  /** How many of a stage's latest features must pass without a unique accepted catch before it is a removal candidate. */
  readonly removal_features: number
}

export interface Routing {
  readonly version: 1
  readonly clients: Readonly<Record<string, CriticClient>>
  /** Per client, the clients tried in order when it may not run. */
  readonly fallbacks: Readonly<Record<string, readonly string[]>>
  readonly stages: Readonly<Partial<Record<CritiqueStage, StageRouting>>>
  /** Read by the stats command only, so a routing without them still prepares runs. */
  readonly stats?: StatsThresholds
}

/** A critic resolved for one run: the slot plus the client it runs through. */
export interface RosterCritic extends RosterSlot {
  readonly critic_id: string
  readonly client_config: CriticClient
}

export const DROP_REASONS = ['family-not-allowed', 'orchestrator-family', 'same-family', 'unavailable'] as const

/** A routed critic that does not run, and the fallback critic that runs in its slot, if one could. */
export interface DroppedCritic {
  readonly client: string
  readonly family: ModelFamily
  readonly reason: (typeof DROP_REASONS)[number]
  /** Why an unavailable client cannot run on this machine. */
  readonly detail?: string
  /** The critic id of the fallback that took the slot. */
  readonly replaced_by?: string
  /** True for a critic of the deep roster, which is left out rather than leaving the base short. */
  readonly deep?: true
}

export interface Roster {
  readonly depth: Depth
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

export function parseRouting(text: string): Routing {
  const file = JsonFields.parse(text)
  return {
    clients: file.entries('clients', readCriticClient),
    fallbacks: file.has('fallbacks') ? file.stringLists('fallbacks') : {},
    stages: file.entries('stages', (stage, name) => ({
      base: stage.list('base', readRosterSlot),
      deep: stage.has('deep') ? stage.list('deep', readRosterSlot) : [],
      depth_signals: stage.has('depth_signals') ? readDepthSignalsConfig(stage.object('depth_signals'), name) : NO_DEPTH_SIGNALS,
      distinct_families: stage.boolean('distinct_families'),
    })),
    version: file.literal('version', 1),
    ...(file.has('stats') ? { stats: readStatsThresholds(file.object('stats')) } : {}),
  }
}

function readStatsThresholds(fields: JsonFields): StatsThresholds {
  return {
    min_adjudicated_findings: fields.positiveInteger('min_adjudicated_findings'),
    removal_features: fields.positiveInteger('removal_features'),
  }
}

export function readCriticClient(fields: JsonFields): CriticClient {
  if (fields.oneOf('kind', CLIENT_KINDS) === 'agy') {
    return {
      binary: fields.string('binary'),
      family: fields.literal('family', 'google'),
      kind: 'agy',
      model: fields.matching('model', isGeminiModel, "a Gemini model: agy's Claude and GPT-OSS models are never critics"),
    }
  }
  return {
    agent: fields.string('agent'),
    binary: fields.string('binary'),
    family: fields.oneOf('family', MODEL_FAMILIES),
    kind: 'vibe',
    model: fields.string('model'),
  }
}

function isGeminiModel(model: string): model is string {
  return model.startsWith('gemini-')
}

export function readRosterSlot(fields: JsonFields): RosterSlot {
  return {
    client: fields.string('client'),
    lenses: fields.strings('lenses'),
    max_price_usd: fields.number('max_price_usd'),
    role: fields.string('role'),
    time_limit_s: fields.number('time_limit_s'),
  }
}

export function readRosterCritic(fields: JsonFields): RosterCritic {
  return {
    ...readRosterSlot(fields),
    client_config: readCriticClient(fields.object('client_config')),
    critic_id: fields.string('critic_id'),
  }
}

export function readDroppedCritic(fields: JsonFields): DroppedCritic {
  const detail = fields.optionalString('detail')
  const replacedBy = fields.optionalString('replaced_by')
  return {
    client: fields.string('client'),
    family: fields.oneOf('family', MODEL_FAMILIES),
    reason: fields.oneOf('reason', DROP_REASONS),
    ...(detail === undefined ? {} : { detail }),
    ...(replacedBy === undefined ? {} : { replaced_by: replacedBy }),
    ...(fields.has('deep') && fields.boolean('deep') ? { deep: true as const } : {}),
  }
}

export interface RosterRequest {
  readonly stage: CritiqueStage
  readonly depth: Depth
  readonly allowedFamilies: readonly ModelFamily[]
  readonly orchestratorFamily: OrchestratorFamily
}

/**
 * The clients a stage could run, routed or fallback, whose family the
 * allowlist permits and the orchestrator does not share: the ones whose
 * readiness is worth checking.
 */
export function candidateClients(routing: Routing, request: RosterRequest): readonly (readonly [string, CriticClient])[] {
  const names = new Set(slotsAt(stageRouting(routing, request), request.depth).flatMap(({ slot }) => slotCandidates(routing, slot.client)))
  return [...names]
    .map((name) => [name, clientNamed(routing, request, name)] as const)
    .filter(([, client]) => familyReason(client.family, request) === undefined)
}

/**
 * Resolves a stage's roster at the request's depth: the base slots, then at
 * deep depth the deep slots. Each slot runs its routed client, or
 * else the first of that client's fallbacks that may run, with the slot's
 * role and lenses. A client may not run when the repository's allowlist does
 * not permit its family, when it shares the orchestrator's family (the
 * orchestrator adjudicates its findings), when the stage requires distinct
 * families and a critic already has its family, or when it is not ready on
 * this machine. A slot no client can fill stays empty, so the stage runs with
 * fewer critics rather than two of one family.
 */
export function resolveRoster(
  routing: Routing,
  request: RosterRequest,
  readiness: ReadonlyMap<string, ClientReadiness> = new Map(),
): Roster {
  const stage = stageRouting(routing, request)
  const critics: RosterCritic[] = []
  const dropped: DroppedCritic[] = []
  const refusal = (name: string, client: CriticClient): Pick<DroppedCritic, 'reason' | 'detail'> | undefined => {
    const reason =
      familyReason(client.family, request) ??
      (stage.distinct_families && critics.some((critic) => critic.client_config.family === client.family)
        ? 'same-family'
        : undefined)
    if (reason !== undefined) return { reason }
    const ready = readiness.get(name)
    return ready?.ready === false ? { detail: ready.detail, reason: 'unavailable' } : undefined
  }
  for (const { slot, deep } of slotsAt(stage, request.depth)) {
    const primary = clientNamed(routing, request, slot.client)
    const primaryRefusal = refusal(slot.client, primary)
    const name = slotCandidates(routing, slot.client).find((candidate) =>
      refusal(candidate, clientNamed(routing, request, candidate)) === undefined,
    )
    const chosen =
      name === undefined
        ? undefined
        : { ...slot, client: name, client_config: clientNamed(routing, request, name), critic_id: `c${String(critics.length + 1)}-${name}` }
    if (primaryRefusal !== undefined) {
      dropped.push({
        client: slot.client,
        family: primary.family,
        ...primaryRefusal,
        ...(chosen === undefined ? {} : { replaced_by: chosen.critic_id }),
        ...(deep ? { deep } : {}),
      })
    }
    if (chosen !== undefined) critics.push(chosen)
  }
  return { critics, depth: request.depth, dropped }
}

function slotsAt(stage: StageRouting, depth: Depth): readonly { readonly slot: RosterSlot; readonly deep?: true }[] {
  return [...stage.base.map((slot) => ({ slot })), ...(depth === 'deep' ? stage.deep.map((slot) => ({ deep: true as const, slot })) : [])]
}

/** A stage's routing; a stage routing does not define is an error. */
export function stageRouting(routing: Routing, request: Pick<RosterRequest, 'stage'>): StageRouting {
  const stage = routing.stages[request.stage]
  if (stage === undefined) throw new CritiqueError('invalid-routing', `routing has no stage ${request.stage}`)
  return stage
}

function slotCandidates(routing: Routing, client: string): readonly string[] {
  return [client, ...(routing.fallbacks[client] ?? [])]
}

function clientNamed(routing: Routing, request: RosterRequest, name: string): CriticClient {
  const client = routing.clients[name]
  if (client === undefined) {
    throw new CritiqueError('invalid-routing', `routing stage ${request.stage} names unknown client ${name}`)
  }
  return client
}

function familyReason(family: ModelFamily, request: RosterRequest): DroppedCritic['reason'] | undefined {
  if (!request.allowedFamilies.includes(family)) return 'family-not-allowed'
  if (family === request.orchestratorFamily) return 'orchestrator-family'
  return undefined
}
