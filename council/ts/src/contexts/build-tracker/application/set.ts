import { applyAssignments, parseAssignments } from '../domain/assignments.js'
import type { BuildTrackerCommand, BuildTrackerDependencies } from './ports.js'
import { loadState, writeState } from './state-file.js'

export interface SetInput {
  readonly state: string
  /** Each `<ticket>=<status>[:note]`, as typed. */
  readonly assignments: readonly string[]
}

export interface SetResult {
  readonly status: 'set'
  readonly changed: readonly { readonly ticket: number; readonly status: string }[]
}

/** Changes the named tickets' statuses in the state file, every one or none. */
export class SetCommand implements BuildTrackerCommand<SetInput, SetResult> {
  constructor(private readonly deps: BuildTrackerDependencies) {}

  async run(input: SetInput): Promise<SetResult> {
    const { raw, state } = await loadState(this.deps.files, input.state)
    const assignments = parseAssignments(input.assignments, state)
    await writeState(this.deps.files, input.state, applyAssignments(raw, assignments))
    return {
      changed: assignments.map((assignment) => ({ status: assignment.status, ticket: assignment.ticket })),
      status: 'set',
    }
  }
}
