import { InitCommand, type InitInput, type InitResult } from './init.js'
import { PathCommand, type PathInput, type PathResult } from './path.js'
import type { BuildTrackerCommand, BuildTrackerDependencies } from './ports.js'
import { RefreshCommand, type RefreshInput, type RefreshResult } from './refresh.js'
import { RenderCommand, type RenderInput, type RenderResult } from './render.js'
import { SetCommand, type SetInput, type SetResult } from './set.js'
import { SyncCommand, type SyncInput, type SyncResult } from './sync.js'

/** The build tracker's commands. */
export interface BuildTrackerCommands {
  readonly init: BuildTrackerCommand<InitInput, InitResult>
  readonly sync: BuildTrackerCommand<SyncInput, SyncResult>
  readonly path: BuildTrackerCommand<PathInput, PathResult>
  readonly refresh: BuildTrackerCommand<RefreshInput, RefreshResult>
  readonly render: BuildTrackerCommand<RenderInput, RenderResult>
  readonly set: BuildTrackerCommand<SetInput, SetResult>
}

export function createBuildTrackerCommands(deps: BuildTrackerDependencies): BuildTrackerCommands {
  return {
    init: new InitCommand(deps),
    path: new PathCommand(deps),
    refresh: new RefreshCommand(deps),
    render: new RenderCommand(deps),
    set: new SetCommand(deps),
    sync: new SyncCommand(deps),
  }
}
