import { randomBytes } from 'node:crypto'

import { usd } from '../domain/caps.js'
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
  type GrantView,
  type LapsedGrant,
} from '../domain/grants.js'
import type { LedgerEntry } from '../domain/ledger.js'
import { requireWorkflowName, requireWorkflowOpen } from '../domain/workflows.js'
import { CapGuard } from './caps.js'
import { loadConfig } from './config.js'
import { messageOf } from './json-file.js'
import { overridePath } from './locations.js'
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

/**
 * Writes the grant the owner gave: the agent asks first, then records the
 * question and the answer here. It refuses an empty question or answer, a cap
 * that is not above 0, a client that is not metered-employer, and a workflow
 * that ended. The grant lapses 24 hours after it was written, or when its
 * workflow's own 24 hours run out first.
 */
export class GrantWriteCommand implements ModelGatewayCommand<unknown, GrantWriteResult> {
  private readonly grants: GrantGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.grants = new GrantGuard(deps)
  }

  async run(raw: unknown): Promise<GrantWriteResult> {
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
    const standing = await this.standing(input.workflow, input.client, now)
    if (standing !== undefined) {
      const room = grantRoom(await this.deps.ledger.read(), standing)
      throw new ModelGatewayError(
        'grant-exists',
        `workflow ${input.workflow} already has live grant ${standing.id} of ${usd(standing.cap_usd)} with ${usd(room.remaining_usd)} left: the owner re-approves only after it lapsed`,
      )
    }
    const grant = newGrant({
      ...input,
      id: grantId(now, randomBytes(3).toString('hex')),
      workflowRecord: record,
      written_at: now,
    })
    const written = await this.deps.grants.update((grants) => ({ grant, grants: [...grants, grant] }))
    return await this.report(written)
  }

  /**
   * The live grant that already covers this workflow and client, or undefined
   * when there is none or the ledger cannot be read: a ledger the gateway
   * cannot read is no reason to refuse the grant the owner gave.
   */
  private async standing(workflow: string, client: string, now: Date): Promise<Grant | undefined> {
    try {
      return await this.grants.live(workflow, client, now)
    } catch (error) {
      if (error instanceof ModelGatewayError) throw error
      return undefined
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

/**
 * The grant a metered-employer run needs. A run whose workflow no live grant
 * covers is refused with `no-grant`, naming the workflow and the `grant write`
 * call that would add one; the gateway never writes or renews a grant itself.
 */
export class GrantGuard {
  private readonly caps: CapGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
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

/** `grant list` takes no field: it lists every grant this machine holds. */
function readListInput(raw: unknown): void {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
}
