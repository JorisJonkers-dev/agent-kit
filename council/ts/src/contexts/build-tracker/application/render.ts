import { resolve } from 'node:path'

import type { Counts } from '../domain/board.js'
import { stateBase, stateLocation, type TicketFiles } from '../domain/spec.js'
import type { BuildTrackerState } from '../domain/state.js'
import type { BuildTrackerCommand, BuildTrackerDependencies } from './ports.js'
import { localSpecPlace } from './specs.js'
import { loadState, writePage } from './state-file.js'

export interface RenderInput {
  readonly state: string
  readonly out: string
}

export interface RenderResult {
  readonly status: 'rendered'
  readonly out: string
  readonly counts: Counts
}

/** Writes the build tracker page for a state file. */
export class RenderCommand implements BuildTrackerCommand<RenderInput, RenderResult> {
  constructor(private readonly deps: BuildTrackerDependencies) {}

  async run(input: RenderInput): Promise<RenderResult> {
    const { state } = await loadState(this.deps.files, input.state)
    const counts = await writePage(this.deps, input.out, state, await this.ticketFiles(input.state, state))
    return { counts, out: input.out, status: 'rendered' }
  }

  /**
   * Where a local spec's ticket files are shown, read from the repository
   * the command runs in. Only the spec's own repository will do: its state
   * file is the one that repository's spec names, so a state rendered from
   * anywhere else gets no links rather than links into the wrong tree.
   */
  private async ticketFiles(statePath: string, state: BuildTrackerState): Promise<TicketFiles | undefined> {
    if (state.spec.kind !== 'local') return undefined
    const base = stateBase(this.deps.env)
    const location = await this.deps.repositories.locate(this.deps.cwd)
    if (base === undefined || location === undefined) return undefined
    const place = localSpecPlace(location)
    const own = stateLocation(base, { kind: 'local', path: state.spec.path, repository: place.repository }).state
    return own === resolve(this.deps.cwd, statePath) ? place.files : undefined
  }
}
