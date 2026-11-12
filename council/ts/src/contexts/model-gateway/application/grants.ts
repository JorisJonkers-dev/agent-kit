import { createHash, randomBytes } from 'node:crypto'
import { join } from 'node:path'

import { fits, reservationRecord, settleExpired, usd } from '../domain/caps.js'
import { isObject, ModelGatewayError } from '../domain/config.js'
import {
  grantId,
  grantLapse,
  grantRoom,
  grantViews,
  liveGrant,
  newGrant,
  noGrantMessage,
  type Grant,
  type GrantRoom,
  type GrantView,
  type LapsedGrant,
} from '../domain/grants.js'
import { isReservation, isSettled, type LedgerEntry } from '../domain/ledger.js'
import { requireWorkflowName, requireWorkflowOpen, type WorkflowRecord } from '../domain/workflows.js'
import { CapGuard } from './caps.js'
import { loadConfig } from './config.js'
import { messageOf } from './json-file.js'
import { ledgerPath, overridePath, stateDir } from './locations.js'
import { DELEGATION_MARK } from './launchers.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** What `grant write` takes: the workflow the owner allowed, the client, the cap, and what was asked and answered. */
export interface GrantWriteInput {
  readonly workflow: string
  readonly client: string
  readonly cap_usd: number
  readonly question: string
  readonly answer: string
}

/** The grant `grant write` wrote, with what it has left; `room` is null when the ledger could not be read. */
export interface GrantWriteResult {
  readonly status: 'ok'
  readonly grant: GrantView | (Grant & { readonly room: null })
  /** Why the grant's room is unknown, when the ledger could not be read after the write. */
  readonly warning?: string
}

/** Every grant this machine holds: the ones that still cover a run, and the ones that lapsed with why. */
export interface GrantListResult {
  readonly status: 'ok'
  readonly grants: readonly GrantView[]
  readonly lapsed: readonly LapsedGrant[]
}

/** What `grant reserve` takes: the run's workflow, the client the node starts, and the node that asks. */
export interface GrantReserveInput {
  readonly workflow: string
  readonly client: string
  readonly node: string
  /** The node's attempt, 1 unless the caller says; one node and attempt holds one start however often it asks. */
  readonly attempt: number
}

/** One Codex node start held against a grant, and what the grant has left in node starts. */
export interface NodeStartReservation {
  readonly run_id: string
  readonly workflow: string
  readonly client: string
  readonly node: string
  readonly attempt: number
  /** True when this node and attempt already held its start, so nothing more was reserved. */
  readonly repeated: boolean
  /** The grant the start is held against. */
  readonly grant: string
  readonly reserved_at: string
  /** The grant's cap, in node starts. */
  readonly cap: number
  /** The starts the grant has spent, this one not among them yet. */
  readonly spent: number
  /** The starts the grant holds, this one included. */
  readonly reserved: number
  /** The starts the grant has left, this one taken. */
  readonly remaining: number
}

/** The start `grant reserve` holds, and what its grant has left. */
export interface GrantReserveResult {
  readonly status: 'ok'
  readonly reservation: NodeStartReservation
}

/** What one Codex node start holds against a Gold Band grant: one unit of its cap, for the minute it holds. */
const NODE_START_ESTIMATE_USD = 1
const NODE_START_TIME_LIMIT_S = 1

/**
 * Writes the grant the owner gave: the agent asks first, then records the
 * question and the answer here. It refuses an empty question or answer, a cap
 * that is not above 0, a client that is not metered-employer, a workflow
 * that ended, a call from inside a delegation, and a ledger it cannot read.
 * The grant lapses 24 hours after it was written, or when its
 * workflow's own 24 hours run out first.
 */
export class GrantWriteCommand implements ModelGatewayCommand<unknown, GrantWriteResult> {
  private readonly caps: CapGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.caps = new CapGuard(deps)
  }

  async run(raw: unknown): Promise<GrantWriteResult> {
    this.refuseNested()
    const input = readGrantInput(raw)
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const client = config.clients[input.client]
    if (client === undefined) throw new ModelGatewayError('unknown-model', `the gateway knows no client ${input.client}`)
    if (client.cost_class !== 'metered-employer') {
      throw new ModelGatewayError(
        'invalid-input',
        `${input.client} is ${client.cost_class}: a grant is for a metered-employer client, which is the one that spends someone else's money`,
      )
    }
    const now = this.deps.clock.now()
    const record = requireWorkflowOpen(await this.deps.workflows.read(), input.workflow, now)
    const entries = await this.readLedger(now)
    const grant = newGrant({
      ...input,
      id: grantId(now, randomBytes(3).toString('hex')),
      workflowRecord: record,
      written_at: now,
    })
    // The duplicate check runs inside the grants lock, so two concurrent writes cannot both pass it.
    const written = await this.deps.grants.update((grants) => {
      const standing = liveGrant(grants, input.workflow, input.client, entries, record, now)
      if (standing !== undefined) {
        const room = grantRoom(entries, standing)
        throw new ModelGatewayError(
          'grant-exists',
          `workflow ${input.workflow} already has live grant ${standing.id} of ${usd(standing.cap_usd)} with ${usd(room.remaining_usd)} left: the owner re-approves only after it lapsed`,
        )
      }
      return { grant, grants: [...grants, grant] }
    })
    return await this.report(written)
  }

  /** A delegate may not write a grant: the mark is in its environment, so this call came from inside a gateway run. */
  private refuseNested(): void {
    if (this.deps.env.get(DELEGATION_MARK) !== undefined) {
      throw new ModelGatewayError(
        'nested-delegation',
        'a delegate may not write a grant: only the agent the owner works in may, after asking the owner',
      )
    }
  }

  /**
   * The ledger, after the reservations whose runs died are settled when the
   * ledger can be written. A ledger that cannot be read refuses the write: a standing grant cannot be ruled
   * out, and a second grant carries a fresh full cap of the employer's money.
   */
  private async readLedger(now: Date): Promise<readonly LedgerEntry[]> {
    // Settling writes the ledger: a ledger that reads fine but cannot be written is no reason to refuse.
    await this.caps.settleStale(now).catch(() => undefined)
    try {
      return await this.deps.ledger.read()
    } catch (error) {
      throw new ModelGatewayError(
        'ledger-unreadable',
        `${ledgerPath(this.deps.env)}: the ledger cannot be read, so a standing grant cannot be ruled out and no grant is written: ${messageOf(error)}`,
      )
    }
  }

  /**
   * The grant it wrote, with what it has left. The write has already happened,
   * so a ledger that cannot be read is a warning rather than a failure: an
   * agent that saw an error would ask the owner again and write a second grant
   * with a fresh cap.
   */
  private async report(grant: Grant): Promise<GrantWriteResult> {
    try {
      return { grant: viewOf(grant, await this.deps.ledger.read()), status: 'ok' }
    } catch (error) {
      return {
        grant: { ...grant, room: null },
        status: 'ok',
        warning: `the grant is written, but the ledger could not be read, so what it has left is unknown: ${messageOf(error)}`,
      }
    }
  }
}

/**
 * Reserves one Codex node start against a Gold Band run's grant, before the
 * node starts: a Gold Band grant's `cap_usd` counts node starts, one per
 * reservation, so the reservation is what lets the node start at all.
 *
 * The decision and the reservation line are one locked ledger update that also
 * settles the reservations whose runs died, so two parallel nodes cannot both
 * pass on the grant's last start. The reservation holds for about a minute and
 * then settles at its full estimate, so a start is never given back. The
 * workflow need not have a record: a Gold Band run is not necessarily a
 * workflow this machine started, and one that ended lapses its grant.
 */
export class GrantReserveCommand implements ModelGatewayCommand<unknown, GrantReserveResult> {
  private readonly caps: CapGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.caps = new CapGuard(deps)
  }

  async run(raw: unknown): Promise<GrantReserveResult> {
    const input = readReserveInput(raw)
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const client = config.clients[input.client]
    if (client === undefined) throw new ModelGatewayError('unknown-model', `the gateway knows no client ${input.client}`)
    if (client.cost_class !== 'metered-employer') {
      throw new ModelGatewayError(
        'invalid-input',
        `${input.client} is ${client.cost_class}: a grant is for a metered-employer client, which is the one that spends someone else's money`,
      )
    }
    const now = this.deps.clock.now()
    // The grants, the workflow record and the finished runs are read before the ledger's lock is taken, so the lock is held for the decision alone.
    const grants = await this.deps.grants.read()
    const records = await this.deps.workflows.read()
    const record = records.find((candidate) => candidate.name === input.workflow)
    const costs = await this.caps.finishedCosts()
    const runId = startId(input)
    const reservation = reservationRecord({
      client: input.client,
      cost_class: 'metered-employer',
      estimate_usd: NODE_START_ESTIMATE_USD,
      model: 'node-start',
      reserved_at: now,
      // A start has no run directory of its own, and a path that holds no result is one no settle reads a cost from.
      run_dir: join(stateDir(this.deps.env), 'node-starts', runId),
      run_id: runId,
      time_limit_s: NODE_START_TIME_LIMIT_S,
      workflow: input.workflow,
    })
    return await this.deps.ledger.update<GrantReserveResult>((entries) => {
      const settled = settleExpired(entries, now, costs)
      const all = [...entries, ...settled]
      // A node that asks again for its own attempt (an agent may run its wrapper twice) already holds its start, held or settled: nothing more is taken.
      const repeated = all.some((entry) => (isReservation(entry) || isSettled(entry)) && entry.run_id === runId)
      const grant = startGrant(grants, input, all, record, now, repeated)
      const room = grantRoom(all, grant)
      if (repeated) {
        return { append: settled, result: startResult(grant, input, runId, reservation.reserved_at, room, true) }
      }
      const start = { limit_usd: grant.cap_usd, reserved_usd: room.reserved_usd, spent_usd: room.spent_usd }
      if (!fits(start, NODE_START_ESTIMATE_USD)) {
        throw new ModelGatewayError(
          'cap-exceeded',
          `grant ${grant.id} allows ${String(grant.cap_usd)} Codex node starts for workflow ${input.workflow}: ${String(room.spent_usd)} started, ${String(room.reserved_usd)} reserved, no start left for node ${input.node}`,
        )
      }
      const held = grantRoom([...all, reservation], grant)
      return { append: [...settled, reservation], result: startResult(grant, input, runId, reservation.reserved_at, held, false) }
    })
  }
}

/** What `grant reserve` prints: the start held, and the grant's room after it in node starts. */
function startResult(
  grant: Grant,
  input: GrantReserveInput,
  runId: string,
  reservedAt: string,
  room: GrantRoom,
  repeated: boolean,
): GrantReserveResult {
  return {
    reservation: {
      attempt: input.attempt,
      cap: grant.cap_usd,
      client: input.client,
      grant: grant.id,
      node: input.node,
      remaining: room.remaining_usd,
      repeated,
      reserved: room.reserved_usd,
      reserved_at: reservedAt,
      run_id: runId,
      spent: room.spent_usd,
      workflow: input.workflow,
    },
    status: 'ok',
  }
}

/** The id of the one start a workflow's node and attempt may hold: the same node asking again finds its own reservation. */
function startId(input: GrantReserveInput): string {
  const key = JSON.stringify([input.workflow, input.client, input.node, input.attempt])
  return `start-${createHash('sha256').update(key).digest('hex').slice(0, 16)}`
}

/**
 * The grant a node start is held against: the newest live grant for the
 * workflow and client, or a `no-grant` refusal naming the newest grant and why
 * it lapsed, or the `grant write` call that would add one.
 */
function startGrant(
  grants: readonly Grant[],
  input: GrantReserveInput,
  entries: readonly LedgerEntry[],
  record: WorkflowRecord | undefined,
  now: Date,
  repeated: boolean,
): Grant {
  const newest = newestGrant(grants, input.workflow, input.client)
  if (newest === undefined) throw new ModelGatewayError('no-grant', noGrantMessage(input.workflow, input.client))
  const lapse = grantLapse(newest, grantRoom(entries, newest), record, now)
  // A node that already holds its start is not refused because that start (or the one before it) spent the cap.
  if (lapse === undefined || (repeated && lapse === 'cap-spent')) return newest
  const live = liveGrant(grants, input.workflow, input.client, entries, record, now)
  if (live === undefined) {
    throw new ModelGatewayError(
      'no-grant',
      `grant ${newest.id} for workflow ${input.workflow} has lapsed (${lapse}): ask the owner for a new grant`,
    )
  }
  return live
}

/** The grant the owner wrote last for a workflow and client, live or not; undefined when there is none. */
function newestGrant(grants: readonly Grant[], workflow: string, client: string): Grant | undefined {
  let newest: Grant | undefined
  for (const grant of grants) {
    if (grant.workflow !== workflow || grant.client !== client) continue
    if (newest === undefined || Date.parse(grant.written_at) >= Date.parse(newest.written_at)) newest = grant
  }
  return newest
}

/**
 * Lists every grant this machine holds: the live ones with what each has left
 * (its cap minus what its workflow spent since the grant minus what live
 * reservations hold), and the lapsed ones with why. It settles the
 * reservations whose runs died first, so a report never shows a reservation as
 * held after the moment it lapsed.
 */
export class GrantListCommand implements ModelGatewayCommand<unknown, GrantListResult> {
  private readonly caps: CapGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.caps = new CapGuard(deps)
  }

  async run(raw: unknown): Promise<GrantListResult> {
    readListInput(raw)
    const now = this.deps.clock.now()
    await this.caps.settleStale(now)
    const views = grantViews(await this.deps.grants.read(), await this.deps.ledger.read(), await this.deps.workflows.read(), now)
    return { grants: views.grants, lapsed: views.lapsed, status: 'ok' }
  }
}

/** What the grant guard reads: the grants, the workflows they lapse with, and the ledger that says what they spent. */
export type GrantGuardDependencies = Pick<ModelGatewayDependencies, 'files' | 'grants' | 'ledger' | 'workflows'>

/**
 * The grant a metered-employer run needs. A run whose workflow no live grant
 * covers is refused with `no-grant`, naming the workflow and the `grant write`
 * call that would add one; the gateway never writes or renews a grant itself.
 */
export class GrantGuard {
  private readonly caps: CapGuard

  constructor(private readonly deps: GrantGuardDependencies) {
    this.caps = new CapGuard(deps)
  }

  /**
   * The live grant that covers a workflow and client now, or undefined when
   * every grant of it lapsed. It settles the reservations whose runs died
   * first, as `grant list` does, so a dead run's reservation never makes a
   * grant look spent for the run that asks next.
   */
  async live(workflow: string, client: string, now: Date): Promise<Grant | undefined> {
    await this.caps.settleStale(now)
    const record = (await this.deps.workflows.read()).find((candidate) => candidate.name === workflow)
    return liveGrant(await this.deps.grants.read(), workflow, client, await this.deps.ledger.read(), record, now)
  }

  /** The clients a live grant covers for a workflow now, which is what lets their models into the fall-through pool. */
  async liveClients(workflow: string, now: Date): Promise<ReadonlySet<string>> {
    await this.caps.settleStale(now)
    const record = (await this.deps.workflows.read()).find((candidate) => candidate.name === workflow)
    const entries = await this.deps.ledger.read()
    const clients = new Set<string>()
    for (const grant of await this.deps.grants.read()) {
      if (grant.workflow !== workflow) continue
      if (grantLapse(grant, grantRoom(entries, grant), record, now) !== undefined) continue
      clients.add(grant.client)
    }
    return clients
  }

  /** The live grant that covers a workflow and client, or a `no-grant` refusal naming the call that would add one. */
  async require(workflow: string, client: string, now: Date): Promise<Grant> {
    const grant = await this.live(workflow, client, now)
    if (grant === undefined) throw new ModelGatewayError('no-grant', noGrantMessage(workflow, client))
    return grant
  }
}

/** One grant as a command reports it: what it allows, and what it has left. */
function viewOf(grant: Grant, entries: readonly LedgerEntry[]): GrantView {
  const room = grantRoom(entries, grant)
  return { ...grant, remaining_usd: room.remaining_usd, reserved_usd: room.reserved_usd, spent_usd: room.spent_usd }
}

function readGrantInput(raw: unknown): GrantWriteInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  const cap = raw.cap_usd
  if (typeof cap !== 'number' || !Number.isFinite(cap) || cap <= 0) {
    throw new ModelGatewayError('invalid-input', 'cap_usd must be a number above 0')
  }
  return {
    answer: requireText(raw.answer, 'answer'),
    cap_usd: cap,
    client: requireText(raw.client, 'client'),
    question: requireText(raw.question, 'question'),
    workflow: requireWorkflowName(raw.workflow, 'a grant must name its workflow'),
  }
}

/** A field a grant must name, refused when it is missing or blank. */
function requireText(value: unknown, name: string): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new ModelGatewayError('invalid-input', `a grant must name its ${name}`)
  }
  return value
}

/** What `grant reserve` takes: the run's workflow, the client, and the node that asks for a start. */
function readReserveInput(raw: unknown): GrantReserveInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  const node = raw.node
  if (typeof node !== 'string' || node.trim() === '') {
    throw new ModelGatewayError('invalid-input', 'a node start must name its node')
  }
  const attempt = raw.attempt ?? 1
  if (typeof attempt !== 'number' || !Number.isInteger(attempt) || attempt < 0) {
    throw new ModelGatewayError('invalid-input', 'attempt must be a whole number, 0 or more')
  }
  return {
    attempt,
    client: requireText(raw.client, 'client'),
    node,
    workflow: requireWorkflowName(raw.workflow, 'a node start must name its workflow'),
  }
}

/** `grant list` takes no field: it lists every grant this machine holds. */
function readListInput(raw: unknown): void {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
}
