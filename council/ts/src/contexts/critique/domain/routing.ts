import { readLimits, type CritiqueLimits } from './budget.js'
import { NO_DEPTH_SIGNALS, readDepthSignalsConfig, type DepthSignalsConfig } from './depth.js'
import { CRITIC_FAMILIES, MODEL_FAMILIES, type AllowlistFamily, type CriticFamily, type ModelFamily, type OrchestratorFamily } from './family.js'
import { JsonFields, JsonShapeError } from './json.js'
import { readTokenPrices, type TokenPrices } from './pricing.js'
import { CritiqueError } from './outcomes.js'
import type { CritiqueStage, Depth } from './stage.js'
import { DEFAULT_PACKET_LIMITS, readPacketLimits, type PacketLimits } from './repository-packet.js'
import { DEFAULT_TICKET_SIZE, type TicketSize } from './tickets.js'
import { readGateRouting, type GateRouting } from './gate.js'
import { readTriageRules, type TriageModel, type TriageRules } from './triage.js'

/**
 * The gateway client model a critic client is pinned to: the client the
 * gateway runs the critic on, and the alias the gateway lists that model by.
 * The critic's own `model` stays what its agent file and the read-only proof
 * name; this is what the gateway is asked for.
 */
export interface GatewayModel {
  readonly client: CriticClient['kind']
  readonly model: string
}

/** One CLI pinned to one model. Model names live here and in the critic agent files only. */
export interface VibeCriticClient {
  readonly kind: 'vibe'
  readonly binary: string
  /** The read-only `critic-<model>` Vibe agent that pins the model and its prices. */
  readonly agent: string
  readonly model: string
  readonly family: ModelFamily
  /**
   * The model's prices, the same as its agent file's, for the runner to
   * compute a launch's cost from its tokens. Without them the runner takes
   * Vibe's own figure.
   */
  readonly prices?: TokenPrices
  /** The gateway client model this critic client is pinned to; without it the client runs its own kind and model. */
  readonly gateway?: GatewayModel
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
  /** The gateway client model this critic client is pinned to; without it the client runs its own kind and model. */
  readonly gateway?: GatewayModel
}

/**
 * Codex on the nedap account. Only the work roster of a `nedap` repository
 * names it, and only under a live model-gateway grant (ADR-0008).
 */
export interface CodexCriticClient {
  readonly kind: 'codex'
  readonly binary: string
  readonly model: string
  readonly family: 'openai'
  /** The gateway client model this critic client is pinned to; without it the client runs its own kind and model. */
  readonly gateway?: GatewayModel
}

/** The work Claude agent. Only the work roster of a `nedap` repository names it. */
export interface ClaudeCriticClient {
  readonly kind: 'claude'
  readonly binary: string
  readonly model: string
  readonly family: 'anthropic'
  /** The gateway client model this critic client is pinned to; without it the client runs its own kind and model. */
  readonly gateway?: GatewayModel
}

export type CriticClient = VibeCriticClient | AgyCriticClient | CodexCriticClient | ClaudeCriticClient

export const CLIENT_KINDS = ['vibe', 'agy', 'codex', 'claude'] as const satisfies readonly CriticClient['kind'][]

/** The client kinds only a `nedap` repository's work roster may use. */
export const WORK_CLIENT_KINDS = ['codex', 'claude'] as const satisfies readonly CriticClient['kind'][]

/** Whether a client belongs to the work roster, and so to no other. */
export function isWorkClient(client: CriticClient): client is CodexCriticClient | ClaudeCriticClient {
  return client.kind === 'codex' || client.kind === 'claude'
}

/** Whether a critic client can run on this machine now; `detail` says why not, in setup's words where setup has them. */
export type ClientReadiness = { readonly ready: true } | { readonly ready: false; readonly detail: string }

/**
 * Whether a client may run in this run: ready, and within its quota. A
 * client past the Antigravity quota is set up but may not be launched today.
 */
export type ClientAvailability = ClientReadiness | { readonly ready: false; readonly detail: string; readonly reason: 'quota' }

/** Routing's `triage`, with the Vibe client its model runs through. */
export interface TriageRouting extends TriageRules {
  readonly model: TriageModel & { readonly client_config: VibeCriticClient }
}

/** The triage model asks a yes/no question through Vibe, so its client must be a Vibe client routing defines. */
function readTriageRouting(fields: JsonFields, clients: Readonly<Record<string, CriticClient>>): TriageRouting {
  const rules = readTriageRules(fields)
  const client = clients[rules.model.client]
  if (client?.kind !== 'vibe') {
    throw new JsonShapeError(
      `triage.model.client must name a vibe client of clients; ${rules.model.client} is ${client === undefined ? 'not defined' : 'an agy client'}`,
    )
  }
  return { ...rules, model: { ...rules.model, client_config: client } }
}

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
  /**
   * The stage's work-only roster, which a `nedap` repository uses instead of
   * base and deep: its slots name the Codex and work Claude clients, one
   * critic each, and no other roster may name those clients.
   */
  readonly work: readonly RosterSlot[]
  /**
   * The critics added at either depth, after the base roster and before the
   * deep one, when the artifact holds a decision record; each is subject to
   * the same family rules. Only the decisions stage reads it.
   */
  readonly adr: readonly RosterSlot[]
  /** What makes the stage go deep when the caller names no depth. */
  readonly depth_signals: DepthSignalsConfig
  /** When a ticket is likely too big for one `/implement` context; only the tickets stage reads it. */
  readonly ticket_size: TicketSize
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
  /** The per-flow Vibe cap and the Antigravity quota. */
  readonly limits: CritiqueLimits
  /** How big a repository packet may be, and the paths it never sends. */
  readonly packet: PacketLimits
  /** Ticket-review risk triage: the risk rules and the triage model; routing without it cannot triage. */
  readonly triage?: TriageRouting
  /** What the PR gate reads: the feature branch patterns. Without it, every branch a push updates is gated. */
  readonly gate?: GateRouting
}

/** A client a critic may be retried through. */
export interface RosterFallback {
  readonly client: string
  readonly client_config: CriticClient
}

/** A critic resolved for one run: the slot plus the client it runs through. */
export interface RosterCritic extends RosterSlot {
  readonly critic_id: string
  readonly client_config: CriticClient
  /**
   * The slot's other clients, in routing order, that the allowlist permits,
   * the orchestrator does not share and that are ready here: what fan-out
   * may retry a failed critic through. A retry has none.
   */
  readonly fallbacks: readonly RosterFallback[]
}

export const DROP_REASONS = ['family-not-allowed', 'orchestrator-family', 'same-family', 'unavailable', 'quota', 'no-grant'] as const

/** A routed critic that does not run, and the fallback critic that runs in its slot, if one could. */
export interface DroppedCritic {
  readonly client: string
  readonly family: CriticFamily
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
  readonly distinct_families: boolean
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

export function parseRouting(text: string): Routing {
  const file = JsonFields.parse(text)
  const clients = file.entries('clients', readCriticClient)
  const triage = file.has('triage') ? readTriageRouting(file.object('triage'), clients) : undefined
  return {
    clients,
    fallbacks: file.has('fallbacks') ? openFallbacks(file.stringLists('fallbacks'), clients) : {},
    limits: readLimits(file),
    packet: file.has('packet') ? readPacketLimits(file.object('packet')) : DEFAULT_PACKET_LIMITS,
    stages: file.entries('stages', (stage, name) => ({
      base: onlyOpenClients(stage.list('base', readRosterSlot), clients, `stages.${name}.base`),
      adr: onlyOpenClients(stage.has('adr') ? stage.list('adr', readRosterSlot) : [], clients, `stages.${name}.adr`),
      deep: onlyOpenClients(stage.has('deep') ? stage.list('deep', readRosterSlot) : [], clients, `stages.${name}.deep`),
      work: onlyWorkClients(stage.has('work') ? stage.list('work', readRosterSlot) : [], clients, `stages.${name}.work`),
      depth_signals: stage.has('depth_signals') ? readDepthSignalsConfig(stage.object('depth_signals'), name) : NO_DEPTH_SIGNALS,
      distinct_families: stage.boolean('distinct_families'),
      ticket_size: stage.has('ticket_size') ? readTicketSize(stage.object('ticket_size')) : DEFAULT_TICKET_SIZE,
    })),
    ...(triage === undefined ? {} : { triage }),
    version: file.literal('version', 1),
    ...(file.has('stats') ? { stats: readStatsThresholds(file.object('stats')) } : {}),
    ...(file.has('gate') ? { gate: readGateRouting(file.object('gate')) } : {}),
  }
}

/** A client is open when a base or deep roster may use it: anything but the work clients. */
function isOpen(clients: Readonly<Record<string, CriticClient>>, name: string): boolean {
  const client = clients[name]
  return client === undefined || !isWorkClient(client)
}

function onlyOpenClients(slots: readonly RosterSlot[], clients: Readonly<Record<string, CriticClient>>, where: string): readonly RosterSlot[] {
  const work = slots.find((slot) => !isOpen(clients, slot.client))
  if (work !== undefined) {
    throw new JsonShapeError(`${where} names ${work.client}, a work client: only a stage's work roster may name Codex or the work Claude agent`)
  }
  return slots
}

function onlyWorkClients(slots: readonly RosterSlot[], clients: Readonly<Record<string, CriticClient>>, where: string): readonly RosterSlot[] {
  const open = slots.find((slot) => isOpen(clients, slot.client))
  if (open !== undefined) {
    throw new JsonShapeError(`${where} names ${open.client}, which is not a codex or claude client: a work roster holds nothing else`)
  }
  return slots
}

function openFallbacks(
  fallbacks: Readonly<Record<string, readonly string[]>>,
  clients: Readonly<Record<string, CriticClient>>,
): Readonly<Record<string, readonly string[]>> {
  for (const [client, names] of Object.entries(fallbacks)) {
    const work = [client, ...names].find((name) => !isOpen(clients, name))
    if (work !== undefined) throw new JsonShapeError(`fallbacks.${client} names ${work}, a work client: it has no fallbacks`)
  }
  return fallbacks
}

function readStatsThresholds(fields: JsonFields): StatsThresholds {
  return {
    min_adjudicated_findings: fields.positiveInteger('min_adjudicated_findings'),
    removal_features: fields.positiveInteger('removal_features'),
  }
}

function readTicketSize(fields: JsonFields): TicketSize {
  return { max_criteria: fields.number('max_criteria'), max_modules: fields.number('max_modules') }
}

export function readCriticClient(fields: JsonFields): CriticClient {
  const kind = fields.oneOf('kind', CLIENT_KINDS)
  const gateway = fields.has('gateway') ? { gateway: readGatewayModel(fields.object('gateway'), kind) } : {}
  if (kind === 'codex') {
    return { binary: fields.string('binary'), family: fields.literal('family', 'openai'), kind, model: fields.string('model'), ...gateway }
  }
  if (kind === 'claude') {
    return { binary: fields.string('binary'), family: fields.literal('family', 'anthropic'), kind, model: fields.string('model'), ...gateway }
  }
  if (kind === 'agy') {
    return {
      binary: fields.string('binary'),
      family: fields.literal('family', 'google'),
      kind: 'agy',
      model: fields.matching('model', isGeminiModel, "a Gemini model: agy's Claude and GPT-OSS models are never critics"),
      ...gateway,
    }
  }
  return {
    agent: fields.string('agent'),
    binary: fields.string('binary'),
    family: fields.oneOf('family', MODEL_FAMILIES),
    kind: 'vibe',
    model: fields.string('model'),
    ...(fields.has('prices') ? { prices: readTokenPrices(fields.object('prices')) } : {}),
    ...gateway,
  }
}

/**
 * The gateway client model a critic client names: the alias the gateway lists,
 * and a client of the critic's own kind. Each adapter launches its own client
 * and reads its output by its own kind, so a critic pinned to a gateway client
 * of another kind is refused rather than launched and misread.
 */
function readGatewayModel(fields: JsonFields, kind: CriticClient['kind']): GatewayModel {
  return {
    client: fields.matching('client', (value): value is CriticClient['kind'] => value === kind, `${kind}, the kind of the critic client`),
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
    fallbacks: fields.list('fallbacks', (fallback) => ({
      client: fallback.string('client'),
      client_config: readCriticClient(fallback.object('client_config')),
    })),
  }
}

export function readDroppedCritic(fields: JsonFields): DroppedCritic {
  const detail = fields.optionalString('detail')
  const replacedBy = fields.optionalString('replaced_by')
  return {
    client: fields.string('client'),
    family: fields.oneOf('family', CRITIC_FAMILIES),
    reason: fields.oneOf('reason', DROP_REASONS),
    ...(detail === undefined ? {} : { detail }),
    ...(replacedBy === undefined ? {} : { replaced_by: replacedBy }),
    ...(fields.has('deep') && fields.boolean('deep') ? { deep: true as const } : {}),
  }
}

export interface RosterRequest {
  readonly stage: CritiqueStage
  readonly depth: Depth
  readonly allowedFamilies: readonly AllowlistFamily[]
  readonly orchestratorFamily: OrchestratorFamily
  /** Whether the artifact holds a decision record, which adds the stage's ADR critics. */
  readonly adr?: boolean
}

/**
 * The clients a stage could run, routed or fallback, whose family the
 * allowlist permits and the orchestrator does not share: the ones whose
 * readiness is worth checking.
 */
export function candidateClients(routing: Routing, request: RosterRequest): readonly (readonly [string, CriticClient])[] {
  const names = new Set(slotsAt(stageRouting(routing, request), request).flatMap(({ slot }) => slotCandidates(routing, slot.client)))
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
  readiness: ReadonlyMap<string, ClientAvailability> = new Map(),
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
    if (ready?.ready !== false) return undefined
    return { detail: ready.detail, reason: 'reason' in ready ? ready.reason : 'unavailable' }
  }
  for (const { slot, deep } of slotsAt(stage, request)) {
    const primary = clientNamed(routing, request, slot.client)
    const primaryRefusal = refusal(slot.client, primary)
    const name = slotCandidates(routing, slot.client).find((candidate) =>
      refusal(candidate, clientNamed(routing, request, candidate)) === undefined,
    )
    const fallbacks = slotCandidates(routing, slot.client)
      .filter((candidate) => candidate !== name)
      .map((candidate) => ({ client: candidate, client_config: clientNamed(routing, request, candidate) }))
      .filter(
        (fallback) =>
          familyReason(fallback.client_config.family, request) === undefined && readiness.get(fallback.client)?.ready !== false,
      )
    const chosen =
      name === undefined
        ? undefined
        : {
            ...slot,
            client: name,
            client_config: clientNamed(routing, request, name),
            critic_id: `c${String(critics.length + 1)}-${name}`,
            fallbacks,
          }
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
  return { critics, depth: request.depth, distinct_families: stage.distinct_families, dropped }
}

/** A stage's slots in the order they fill: base, then the ADR critics when the artifact holds a record, then deep at deep depth. */
function slotsAt(stage: StageRouting, request: Pick<RosterRequest, 'adr' | 'depth'>): readonly { readonly slot: RosterSlot; readonly deep?: true }[] {
  return [
    ...stage.base.map((slot) => ({ slot })),
    ...(request.adr === true ? stage.adr.map((slot) => ({ slot })) : []),
    ...(request.depth === 'deep' ? stage.deep.map((slot) => ({ deep: true as const, slot })) : []),
  ]
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

function clientNamed(routing: Routing, request: Pick<RosterRequest, 'stage'>, name: string): CriticClient {
  const client = routing.clients[name]
  if (client === undefined) {
    throw new CritiqueError('invalid-routing', `routing stage ${request.stage} names unknown client ${name}`)
  }
  return client
}

function familyReason(family: CriticFamily, request: RosterRequest): DroppedCritic['reason'] | undefined {
  if (!request.allowedFamilies.includes(family)) return 'family-not-allowed'
  if (family === request.orchestratorFamily) return 'orchestrator-family'
  return undefined
}

/**
 * Whether Codex may run for a `nedap` repository now: a live model-gateway
 * grant covers the run's workflow, or `detail` says what is missing.
 */
export type WorkGrant = { readonly live: true } | { readonly live: false; readonly detail: string }

export interface WorkRosterRequest {
  readonly stage: CritiqueStage
  readonly orchestratorFamily: OrchestratorFamily
  readonly grant: WorkGrant
}

/** Codex needs the grant; the work Claude agent runs on the work account and needs none. */
function workReason(client: CriticClient, request: WorkRosterRequest): Pick<DroppedCritic, 'reason' | 'detail'> | undefined {
  if (client.family === request.orchestratorFamily) return { reason: 'orchestrator-family' }
  if (client.kind === 'codex' && !request.grant.live) return { detail: request.grant.detail, reason: 'no-grant' }
  return undefined
}

/**
 * The work clients of a stage whose family is not the orchestrator's and that
 * the grant lets run: the ones whose readiness is worth checking.
 */
export function workCandidateClients(routing: Routing, request: WorkRosterRequest): readonly (readonly [string, CriticClient])[] {
  return stageRouting(routing, request)
    .work.map((slot) => [slot.client, clientNamed(routing, request, slot.client)] as const)
    .filter(([, client]) => workReason(client, request) === undefined)
}

/**
 * Resolves a `nedap` repository's roster from the stage's work slots: Codex
 * on the nedap account under a live grant, and the work Claude agent, each
 * never in the orchestrator's own family, so a Claude orchestrator gets the
 * Codex critic and a Codex orchestrator the work Claude critic. A slot that
 * may not run is dropped with its reason and has no fallback; the roster
 * holds no family a work slot does not name, and no deep critics.
 */
export function resolveWorkRoster(
  routing: Routing,
  request: WorkRosterRequest,
  readiness: ReadonlyMap<string, ClientAvailability> = new Map(),
): Roster {
  const critics: RosterCritic[] = []
  const dropped: DroppedCritic[] = []
  for (const slot of stageRouting(routing, request).work) {
    const client = clientNamed(routing, request, slot.client)
    const ready = readiness.get(slot.client)
    const refusal =
      workReason(client, request) ??
      (ready?.ready === false ? { detail: ready.detail, reason: 'reason' in ready ? ready.reason : ('unavailable' as const) } : undefined)
    if (refusal !== undefined) {
      dropped.push({ client: slot.client, family: client.family, ...refusal })
      continue
    }
    critics.push({ ...slot, client_config: client, critic_id: `c${String(critics.length + 1)}-${slot.client}`, fallbacks: [] })
  }
  return { critics, depth: 'base', distinct_families: true, dropped }
}
