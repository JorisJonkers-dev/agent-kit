import type { Refusal } from '../domain/outcomes.js'
import { checkTickets, ticketCheckStatus, type TicketCheck, type TicketCheckStatus } from '../domain/tickets.js'
import type { TrackerKind } from '../domain/tracker.js'
import { readInput } from './decode.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'
import { TrackerResolver, type TrackedArtifact, type TrackedTicket, type Tracker } from './tracker.js'

export interface CheckTicketsResult extends TicketCheck {
  /** `failed` while the tickets hold a dependency cycle; everything else is reported without failing. */
  readonly status: TicketCheckStatus
  readonly spec: { readonly tracker: TrackerKind; readonly reference: string }
}

/** A spec and its tickets as the tracker holds them, with what the checker reports about them. */
export interface CheckedTickets {
  readonly status: 'read'
  readonly spec: TrackedArtifact
  readonly tickets: readonly TrackedTicket[]
  readonly result: CheckTicketsResult
}

/**
 * Reads a spec and its tickets through the repository's tracker and checks
 * them. Both the check-tickets command and the tickets stage's prepare read
 * them this way, so the packet's report is what check-tickets prints.
 */
export async function readCheckedTickets(
  tracker: Tracker,
  request: { readonly spec: string; readonly comments: boolean },
): Promise<CheckedTickets | Refusal> {
  const spec = await tracker.artifact({ comments: request.comments, reference: request.spec })
  if (spec.status === 'refused') return spec
  const read = await tracker.tickets(request.spec)
  if (read.status === 'refused') return read
  const check = checkTickets(spec.text, read.tickets)
  return {
    result: { status: ticketCheckStatus(check), spec: { reference: spec.reference, tracker: spec.tracker }, ...check },
    spec,
    status: 'read',
    tickets: read.tickets,
  }
}

/**
 * The deterministic ticket checker: reads a spec and its tickets through the
 * repository's tracker, the way prepare reads a spec, and reports what a
 * script can judge. It sends nothing to any critic and writes nothing, so it
 * needs no family allowlist entry; a repository whose remotes are refused is
 * still refused.
 */
export class CheckTicketsCommand implements CritiqueCommand<CheckTicketsResult | Refusal> {
  private readonly repositories: RepositoryGate
  private readonly trackers: TrackerResolver

  constructor(deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.trackers = new TrackerResolver(deps)
  }

  async run(raw: unknown): Promise<CheckTicketsResult | Refusal> {
    const input = readInput(raw, 'check-tickets', (fields) => ({ repo: fields.string('repo'), spec: fields.string('spec') }))
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const tracker = await this.trackers.resolve(repository)
    if (tracker.status === 'refused') return tracker
    const checked = await readCheckedTickets(tracker, { comments: false, spec: input.spec })
    return checked.status === 'refused' ? checked : checked.result
  }
}
