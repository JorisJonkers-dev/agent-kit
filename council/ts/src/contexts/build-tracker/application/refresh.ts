import { applyAssignments, landedAssignments } from '../domain/assignments.js'
import type { Counts } from '../domain/board.js'
import { seedState, syncState } from '../domain/seed.js'
import { isMerged, parseState } from '../domain/state.js'
import { locateSpec, specPaths, type SpecPaths } from './located-spec.js'
import type { BuildTrackerCommand, BuildTrackerDependencies } from './ports.js'
import { readStateFile, writePage, writeState } from './state-file.js'

export interface RefreshInput {
  /** The spec as typed: an issue reference or a local markdown file. */
  readonly spec: string
  /** The tickets the flow records as landed, each `12`, `#12` or a local ticket's `01`. */
  readonly landed: readonly string[]
}

export interface RefreshResult extends SpecPaths {
  readonly status: 'refreshed'
  /** `init` when the spec had no state, `sync` when it had one. */
  readonly ran: 'init' | 'sync'
  readonly tickets: number
  /** Tickets the state did not have before; every ticket on `init`. */
  readonly added: readonly number[]
  /** Tickets that are merged now and were not before: newly closed, or named as landed. */
  readonly merged: readonly number[]
  readonly counts: Counts
}

/**
 * Brings the spec's page up to date in one step, as /flow runs it: init (no
 * milestones) or sync, then each landed ticket merged, then the page.
 */
export class RefreshCommand implements BuildTrackerCommand<RefreshInput, RefreshResult> {
  constructor(private readonly deps: BuildTrackerDependencies) {}

  async run(input: RefreshInput): Promise<RefreshResult> {
    const located = await locateSpec(input.spec, this.deps)
    const { location, source } = located
    const before = await readStateFile(this.deps.files, location.state)
    const snapshot = await source.snapshot()
    const outcome =
      before === undefined
        ? { added: snapshot.tickets.map((ticket) => ticket.number), raw: seedState(source.spec, snapshot, []) }
        : syncState(before.raw, snapshot)
    const raw = applyAssignments(outcome.raw, landedAssignments(input.landed, parseState(outcome.raw)))
    const state = parseState(raw)
    const wasMerged = new Set(before?.state.tickets.filter(isMerged).map((ticket) => ticket.number))
    // The page first: when it cannot be written, the state stays as it was.
    const counts = await writePage(this.deps, location.page, state, source.files)
    await writeState(this.deps.files, location.state, raw)
    return {
      added: outcome.added,
      counts,
      merged: state.tickets.filter((ticket) => isMerged(ticket) && !wasMerged.has(ticket.number)).map((ticket) => ticket.number),
      ...specPaths(located),
      ran: before === undefined ? 'init' : 'sync',
      status: 'refreshed',
      tickets: state.tickets.length,
    }
  }
}
