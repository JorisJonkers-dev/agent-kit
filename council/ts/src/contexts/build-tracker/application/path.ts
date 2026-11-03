import { locateSpec, specPaths, type SpecPaths } from './located-spec.js'
import type { BuildTrackerCommand, BuildTrackerDependencies } from './ports.js'

export interface PathInput {
  /** The spec as typed: an issue reference or a local markdown file. */
  readonly spec: string
}

/** Where a spec's state file and page live. */
export interface PathResult extends SpecPaths {
  readonly status: 'path'
}

/** Says where the spec's state file and page live, whether or not they exist yet. */
export class PathCommand implements BuildTrackerCommand<PathInput, PathResult> {
  constructor(private readonly deps: BuildTrackerDependencies) {}

  async run(input: PathInput): Promise<PathResult> {
    return { ...specPaths(await locateSpec(input.spec, this.deps)), status: 'path' }
  }
}
