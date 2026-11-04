import { join } from 'node:path'

import { spendByFlow, totalSpend, type LoggedInvocation } from '../domain/budget.js'
import { parseGateRecord, type GateRecord } from '../domain/feature-review.js'
import {
  draftFlow,
  flowState,
  flowStateFileName,
  nextStep,
  parseFlowState,
  type FlowRun,
  type FlowSources,
  type FlowState,
  type NextStep,
} from '../domain/flow.js'
import type { JsonFields } from '../domain/json.js'
import { flowEventLine, type FlowEvent, type Ledger } from '../domain/ledger.js'
import { parseManifest, specReference } from '../domain/manifest.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import { parseTicketReviewRecord, type TicketReviewRecord } from '../domain/ticket-review.js'
import { decoded, readInput, readJsonFile } from './decode.js'
import { gateIndexPath } from './feature-gate.js'
import { LedgerSource } from './ledger-source.js'
import { ledgerPath, manifestPath, reportPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate, type ResolvedRepository } from './repository.js'
import { ticketReviewIndexPath } from './ticket-review.js'
import { TrackerResolver, type TrackedArtifact, type TrackedTicket, type Tracker } from './tracker.js'

/** Where one flow's state is kept: the critique folder of the git common directory, which every worktree shares. */
export function flowStatePath(critiqueDir: string, flow: string): string {
  return join(critiqueDir, 'flows', flowStateFileName(flow))
}

export interface StateResult {
  readonly status: 'read'
  /**
   * `cache` when the saved state was returned as it was, `refreshed` when
   * it was computed again over a saved one, `rebuilt` when there was none.
   */
  readonly source: 'cache' | 'refreshed' | 'rebuilt'
  readonly state_path: string
  readonly state: FlowState
  readonly next: NextStep
}

/** What the owner tells the flow state; each becomes a flow event in the ledger, so a rebuild keeps it. */
interface StateRecord {
  readonly grilled: boolean
  readonly no_critique: boolean | undefined
  readonly landed: readonly string[]
  readonly answered: readonly { readonly run_id: string; readonly finding_id: string }[]
}

interface StateInput extends FlowName {
  readonly cached: boolean
  readonly record: StateRecord | undefined
}

const DRAFT_NAME = /^[A-Za-z0-9][A-Za-z0-9._/-]{0,99}$/

/**
 * Reads, updates and rebuilds one feature's flow state, and says what the
 * single next step is. The state is computed from the ledger, the run
 * directories, the review and gate indexes and the tracker, then saved as a
 * cache other readers can take without recomputing it; nothing in it exists
 * only in the cache. It sends nothing anywhere, so it needs no family
 * allowlist entry, but a repository whose remotes refuse a critique is
 * refused here too.
 */
export class StateCommand implements CritiqueCommand<StateResult | Refusal> {
  private readonly flows: FlowLocator
  private readonly source: LedgerSource

  constructor(private readonly deps: CritiqueDeps) {
    this.flows = new FlowLocator(deps)
    this.source = new LedgerSource(deps)
  }

  async run(raw: unknown): Promise<StateResult | Refusal> {
    const input = readInput(raw, 'state', readStateInput)
    const located = await this.flows.locate(input)
    if (located.status === 'refused') return located
    const { flow, repository, spec, tracker } = located
    const critiqueDir = join(repository.commonDir, 'critique')
    const path = flowStatePath(critiqueDir, flow)
    const saved = await this.deps.files.exists(path)
    if (input.cached && input.record === undefined && saved) {
      const state = await this.cachedState(path)
      if (state !== undefined) return { next: nextStep(state), source: 'cache', state, state_path: path, status: 'read' }
    }
    const tickets = spec === undefined ? [] : await this.tickets(tracker, input.spec ?? '')
    if (!Array.isArray(tickets)) return tickets as Refusal
    const { invocations, ledger } = await this.source.read()
    const { draft, keys } = flowKeys(ledger, located, input.draft)
    const gather = {
      critiqueDir,
      draft: spec === undefined ? null : (draft ?? null),
      flow,
      invocations,
      keys,
      ledger,
      repository,
      spec: spec?.reference ?? null,
      tickets: tickets as readonly TrackedTicket[],
    }
    const before = await this.sources(gather)
    const events = newEvents(input, flowState(before), spec !== undefined && input.draft !== undefined && linkedDraft(ledger, repository.key, flow) !== input.draft)
    await this.recordEvents(events, repository.key, flow)
    const state = flowState({ ...before, events: [...before.events, ...events] })
    await this.deps.files.writeText(path, `${JSON.stringify(state, null, 2)}\n`)
    return { next: nextStep(state), source: saved ? 'refreshed' : 'rebuilt', state, state_path: path, status: 'read' }
  }

  /** The saved state, or undefined when it no longer reads as one, so it is rebuilt. */
  private async cachedState(path: string): Promise<FlowState | undefined> {
    try {
      return parseFlowState(await this.deps.files.readText(path))
    } catch {
      return undefined
    }
  }

  private async tickets(tracker: Tracker, spec: string): Promise<readonly TrackedTicket[] | Refusal> {
    const read = await tracker.tickets(spec)
    return read.status === 'refused' ? read : read.tickets
  }

  private async recordEvents(events: readonly FlowEvent[], repository: string, flow: string): Promise<void> {
    if (events.length === 0) return
    const context = { flow, recorded_at: this.deps.clock.now().toISOString(), repository }
    await this.deps.files.appendText(ledgerPath(this.deps.env), events.map((event) => `${flowEventLine(event, context)}\n`).join(''))
  }

  private async sources(gather: Gather): Promise<FlowSources> {
    const { keys, ledger, repository, spec } = gather
    const belongs = (line: { readonly repository: string; readonly flow?: string | undefined; readonly spec?: string | undefined }) =>
      line.repository === repository.key && ((line.flow !== undefined && keys.includes(line.flow)) || (spec !== null && line.spec === spec))
    const runs = await this.runs(gather, belongs)
    const runIds = new Set(runs.map((run) => run.run_id))
    const reviews = (await this.index(ticketReviewIndexPath(gather.critiqueDir), parseTicketReviewRecord)).filter(
      (review: TicketReviewRecord) => review.repository === repository.key,
    )
    const gates = (await this.index(gateIndexPath(gather.critiqueDir), parseGateRecord)).filter(
      (gate: GateRecord) => gate.repository === repository.key && gate.spec === spec,
    )
    return {
      draft: gather.draft,
      events: ledger.flow_events.filter((event) => event.repository === repository.key && keys.includes(event.flow)),
      findings: ledger.findings.filter((finding) => runIds.has(finding.run_id)),
      flow: gather.flow,
      gates,
      head_commit: await this.deps.git.resolveCommit(repository.root, 'HEAD'),
      now: this.deps.clock.now().toISOString(),
      repository: repository.key,
      reviews,
      runs,
      // Matched to tickets by reference: a triage run without the spec keeps the ticket's own flow.
      skipped_reviews: ledger.skipped_reviews.filter((skip) => skip.repository === repository.key),
      spec,
      tickets: gather.tickets,
      vibe_spend_usd: totalSpend(spendByFlow(gather.invocations, repository.key, keys)),
    }
  }

  /**
   * The flow's runs, from the run directories (which hold runs not yet
   * recorded, and recorded runs no critic took part in) and from the ledger
   * (which still holds runs whose directory is gone).
   */
  private async runs(
    gather: Gather,
    belongs: (line: { readonly repository: string; readonly flow?: string | undefined; readonly spec?: string | undefined }) => boolean,
  ): Promise<readonly FlowRun[]> {
    const runs = new Map<string, FlowRun>()
    for (const name of await this.deps.files.listDirectories(gather.critiqueDir)) {
      const runDir = join(gather.critiqueDir, name)
      if (!(await this.deps.files.exists(manifestPath(runDir)))) continue
      const manifest = await readJsonFile(this.deps.files, manifestPath(runDir), 'invalid-run', parseManifest)
      if (!belongs({ flow: manifest.flow, repository: manifest.repository, spec: specReference(manifest) })) continue
      runs.set(manifest.run_id, {
        recorded: await this.deps.files.exists(reportPath(runDir)),
        round: manifest.round,
        run_id: manifest.run_id,
        stage: manifest.stage,
        ticket_check: manifest.ticket_check?.status ?? null,
      })
    }
    for (const line of gather.ledger.runs.filter(belongs)) {
      const known = runs.get(line.run_id)
      runs.set(line.run_id, known === undefined ? { recorded: true, round: line.round, run_id: line.run_id, stage: line.stage, ticket_check: null } : { ...known, recorded: true })
    }
    return [...runs.values()]
  }

  /** One JSON line per record, oldest first; a line that does not parse is an unreadable run record. */
  private async index<T>(path: string, parse: (line: string) => T): Promise<readonly T[]> {
    if (!(await this.deps.files.exists(path))) return []
    const lines = (await this.deps.files.readText(path)).split('\n').filter((line) => line.trim().length > 0)
    return lines.map((line, n) => decoded('invalid-run', `${path} line ${String(n + 1)}`, () => parse(line)))
  }
}

interface Gather {
  readonly critiqueDir: string
  readonly flow: string
  readonly keys: readonly string[]
  readonly spec: string | null
  readonly draft: string | null
  readonly ledger: Ledger
  readonly invocations: readonly LoggedInvocation[]
  readonly repository: ResolvedRepository
  readonly tickets: readonly TrackedTicket[]
}

/** One feature's flow: its repository and tracker, its spec when it has one, and the key its runs carry. */
export interface LocatedFlow {
  readonly status: 'located'
  readonly repository: ResolvedRepository
  readonly tracker: Tracker
  readonly spec: TrackedArtifact | undefined
  readonly flow: string
}

/** How a caller names a feature: by its spec, or by a draft name before `/to-spec` has written one. */
export interface FlowName {
  readonly repo: string
  readonly spec: string | undefined
  readonly draft: string | undefined
}

/**
 * Finds the flow a caller names, the way every flow command does: a spec,
 * read through the repository's tracker, keys it by its reference, and a
 * draft name by `draft:<name>`. A repository whose remotes refuse a
 * critique, an unsupported tracker and a spec the tracker cannot read are
 * refused.
 */
export class FlowLocator {
  private readonly repositories: RepositoryGate
  private readonly trackers: TrackerResolver

  constructor(deps: Pick<CritiqueDeps, 'env' | 'files' | 'gh' | 'git'>) {
    this.repositories = new RepositoryGate(deps)
    this.trackers = new TrackerResolver(deps)
  }

  async locate(name: FlowName): Promise<LocatedFlow | Refusal> {
    const repository = await this.repositories.resolve(name.repo)
    if (repository.status === 'refused') return repository
    const tracker = await this.trackers.resolve(repository)
    if (tracker.status === 'refused') return tracker
    const spec = name.spec === undefined ? undefined : await tracker.artifact({ comments: false, reference: name.spec })
    if (spec?.status === 'refused') return spec
    return { flow: spec?.reference ?? draftFlow(name.draft ?? ''), repository, spec, status: 'located', tracker }
  }
}

/**
 * The keys a flow's runs and launches carry: its own, and once a spec has a
 * draft (named now, or linked earlier), the draft's too, so the draft's runs
 * and spend count with the spec's.
 */
export function flowKeys(
  ledger: Ledger,
  located: Pick<LocatedFlow, 'flow' | 'repository' | 'spec'>,
  draft: string | undefined,
): { readonly draft: string | undefined; readonly keys: readonly string[] } {
  const linked = draft ?? linkedDraft(ledger, located.repository.key, located.flow)
  return { draft: linked, keys: located.spec === undefined || linked === undefined ? [located.flow] : [located.flow, draftFlow(linked)] }
}

/** The draft a spec flow grew from, as the last `draft` event of the flow names it. */
function linkedDraft(ledger: Ledger, repository: string, flow: string): string | undefined {
  return ledger.flow_events.reduce<string | undefined>(
    (draft, event) => (event.event === 'draft' && event.repository === repository && event.flow === flow ? event.draft : draft),
    undefined,
  )
}

/** The events the input records, each checked against the state as it stood. */
function newEvents(input: StateInput, state: FlowState, link: boolean): readonly FlowEvent[] {
  const record = input.record
  const events: FlowEvent[] = link ? [{ draft: input.draft ?? '', event: 'draft' }] : []
  if (record === undefined) return events
  if (record.grilled) events.push({ event: 'grilled' })
  if (record.no_critique !== undefined) events.push({ event: 'no-critique', value: record.no_critique })
  for (const named of record.landed) {
    const ticket = state.tickets.find((candidate) => candidate.id === named || candidate.reference === named)
    if (ticket === undefined) throw new CritiqueError('invalid-input', `state input: ${named} is not a ticket of ${state.flow}`)
    events.push({ event: 'landed', ticket: ticket.reference })
  }
  for (const answer of record.answered) {
    const open = state.open_questions.some((question) => question.run_id === answer.run_id && question.finding_id === answer.finding_id)
    if (!open) throw new CritiqueError('invalid-input', `state input: ${answer.finding_id} of run ${answer.run_id} is not an open question of ${state.flow}`)
    events.push({ event: 'answered', finding_id: answer.finding_id, run_id: answer.run_id })
  }
  return events
}

function readStateInput(fields: JsonFields): StateInput {
  return {
    ...readFlowName(fields, 'state'),
    cached: fields.has('cached') && fields.boolean('cached'),
    record: fields.has('record') ? readRecord(fields.object('record')) : undefined,
  }
}

/** Reads `repo` with `spec` or `draft`, at least one of the two, for the command `command`. */
export function readFlowName(fields: JsonFields, command: string): FlowName {
  const spec = fields.optionalString('spec')
  const draft = fields.optionalMatching('draft', isDraftName, 'a draft name: letters, digits, dots, dashes, underscores and slashes')
  if (spec === undefined && draft === undefined) {
    throw new CritiqueError('invalid-input', `${command} input: name the feature by its spec, or by a draft name before /to-spec has written one`)
  }
  return { draft, repo: fields.string('repo'), spec }
}

function readRecord(fields: JsonFields): StateRecord {
  return {
    answered: fields.has('answered')
      ? fields.list('answered', (answer) => ({ finding_id: answer.string('finding_id'), run_id: answer.string('run_id') }))
      : [],
    grilled: fields.has('grilled') && fields.boolean('grilled'),
    landed: fields.has('landed') ? fields.strings('landed') : [],
    no_critique: fields.has('no_critique') ? fields.boolean('no_critique') : undefined,
  }
}

function isDraftName(value: string): value is string {
  return DRAFT_NAME.test(value)
}
