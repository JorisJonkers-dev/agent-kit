import { syncState } from '../domain/seed.js'
import { parseState } from '../domain/state.js'
import { locateSpec, specPaths, type SpecPaths } from './located-spec.js'
import type { BuildTrackerCommand, BuildTrackerDependencies } from './ports.js'
import { loadState, writeState } from './state-file.js'

export interface SyncInput {
  /** The spec as typed: an issue reference or a local markdown file. */
  readonly spec: string
}

export interface SyncResult extends SpecPaths {
  readonly status: 'synced'
  readonly tickets: number
  /** Tickets the state did not have before. */
  readonly added: readonly number[]
  /** Tickets marked merged because their issue closed. */
  readonly merged: readonly number[]
}

/** Brings the spec's state up to date with its tickets on the issue tracker, keeping what the agent set. */
export class SyncCommand implements BuildTrackerCommand<SyncInput, SyncResult> {
  constructor(private readonly deps: BuildTrackerDependencies) {}

  async run(input: SyncInput): Promise<SyncResult> {
    const located = await locateSpec(input.spec, this.deps)
    const { raw } = await loadState(this.deps.files, located.location.state)
    const outcome = syncState(raw, await located.source.snapshot())
    const synced = parseState(outcome.raw)
    await writeState(this.deps.files, located.location.state, outcome.raw)
    return {
      added: outcome.added,
      merged: outcome.merged,
      ...specPaths(located),
      status: 'synced',
      tickets: synced.tickets.length,
    }
  }
}
