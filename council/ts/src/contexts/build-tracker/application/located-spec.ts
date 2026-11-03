import { specText, stateBase, stateLocation, type StateLocation } from '../domain/spec.js'
import { BuildTrackerError } from '../domain/state.js'
import type { BuildTrackerDependencies } from './ports.js'
import { resolveSpec, type SpecSource } from './specs.js'

/** A `--spec` argument resolved, with where its state file and page live. */
export interface LocatedSpec {
  readonly source: SpecSource
  readonly location: StateLocation
  /** How the state and the command output name the spec. */
  readonly spec: string
}

/** What every command that takes `--spec` prints about where the spec's files are. */
export interface SpecPaths {
  readonly spec: string
  readonly state: string
  readonly page: string
}

export async function locateSpec(argument: string, deps: BuildTrackerDependencies): Promise<LocatedSpec> {
  const source = await resolveSpec(argument, deps)
  const base = stateBase(deps.env)
  if (base === undefined) {
    throw new BuildTrackerError('no-state-home', 'neither XDG_STATE_HOME nor HOME is an absolute path, so there is no place outside the repository for the state; set one of them')
  }
  return { location: stateLocation(base, source.spec), source, spec: specText(source.spec) }
}

export function specPaths(located: LocatedSpec): SpecPaths {
  return { page: located.location.page, spec: located.spec, state: located.location.state }
}
