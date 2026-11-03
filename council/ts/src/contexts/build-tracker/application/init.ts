import { parseMilestones, seedState } from '../domain/seed.js'
import { BuildTrackerError } from '../domain/state.js'
import { locateSpec, specPaths, type SpecPaths } from './located-spec.js'
import type { BuildTrackerCommand, BuildTrackerDependencies } from './ports.js'
import { readStateFile, writeState } from './state-file.js'

export interface InitInput {
  /** The spec as typed: an issue reference or a local markdown file. */
  readonly spec: string
  /** Each `<name>=<ticket>`, in order. */
  readonly milestones: readonly string[]
}

export interface InitResult extends SpecPaths {
  readonly status: 'initialized'
  readonly tickets: number
  readonly milestones: readonly string[]
}

/**
 * Writes a new state for the spec from its tickets on the issue tracker.
 * It refuses a spec that has a state already, naming sync, and one whose
 * state file it cannot read, saying to delete it first.
 */
export class InitCommand implements BuildTrackerCommand<InitInput, InitResult> {
  constructor(private readonly deps: BuildTrackerDependencies) {}

  async run(input: InitInput): Promise<InitResult> {
    const located = await locateSpec(input.spec, this.deps)
    const { location, source, spec } = located
    if ((await readStateFile(this.deps.files, location.state)) !== undefined) {
      throw new BuildTrackerError('state-exists', `a state for ${spec} already exists at ${location.state}; bring it up to date with sync`)
    }
    const snapshot = await source.snapshot()
    const milestones = parseMilestones(input.milestones, snapshot)
    await writeState(this.deps.files, location.state, seedState(source.spec, snapshot, milestones))
    return {
      milestones: milestones.map((milestone) => milestone.name),
      ...specPaths(located),
      status: 'initialized',
      tickets: snapshot.tickets.length,
    }
  }
}
