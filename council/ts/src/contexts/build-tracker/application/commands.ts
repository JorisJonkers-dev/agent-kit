import { applyAssignments, parseAssignments } from '../domain/assignments.js'
import { buildBoard, type Shown } from '../domain/board.js'
import { renderPage } from '../domain/page.js'
import { BuildTrackerError, parseState, type JsonObject, type TrackerState } from '../domain/state.js'
import type { BuildTrackerClockPort, BuildTrackerFilesPort } from './ports.js'

export interface RenderInput {
  readonly state: string
  readonly out: string
}

export interface RenderResult {
  readonly status: 'rendered'
  readonly out: string
  readonly counts: Readonly<Partial<Record<Shown, number>>>
}

export interface SetInput {
  readonly state: string
  /** Each `<ticket>=<status>[:note]`, as typed. */
  readonly assignments: readonly string[]
}

export interface SetResult {
  readonly status: 'set'
  readonly changed: readonly { readonly ticket: number; readonly status: string }[]
}

export interface BuildTrackerCommands {
  readonly render: { run(input: RenderInput): Promise<RenderResult> }
  readonly set: { run(input: SetInput): Promise<SetResult> }
}

export interface BuildTrackerDependencies {
  readonly files: BuildTrackerFilesPort
  readonly clock: BuildTrackerClockPort
}

export function createBuildTrackerCommands(deps: BuildTrackerDependencies): BuildTrackerCommands {
  return {
    render: {
      async run(input) {
        const { state } = await loadState(deps.files, input.state)
        await deps.files.writeAtomic(input.out, renderPage(state, stamp(deps.clock.now())))
        return { counts: buildBoard(state).counts, out: input.out, status: 'rendered' }
      },
    },
    set: {
      async run(input) {
        const { raw, state } = await loadState(deps.files, input.state)
        const assignments = parseAssignments(input.assignments, state)
        await deps.files.writeAtomic(input.state, `${JSON.stringify(applyAssignments(raw, assignments), null, 2)}\n`)
        return {
          changed: assignments.map((assignment) => ({ status: assignment.status, ticket: assignment.ticket })),
          status: 'set',
        }
      },
    },
  }
}

async function loadState(files: BuildTrackerFilesPort, path: string): Promise<{ raw: JsonObject; state: TrackerState }> {
  let text: string
  try {
    text = await files.readText(path)
  } catch (error) {
    if (errorCode(error) === 'ENOENT') {
      throw new BuildTrackerError('missing-state', `there is no state file at ${path}; create it with init`)
    }
    throw unreadable(path, error instanceof Error ? error.message : String(error))
  }
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch (error) {
    throw unreadable(path, `it is not JSON (${String(error)})`)
  }
  try {
    const state = parseState(raw)
    return { raw: raw as JsonObject, state }
  } catch (error) {
    throw unreadable(path, (error as Error).message)
  }
}

function unreadable(path: string, reason: string): BuildTrackerError {
  return new BuildTrackerError('unreadable-state', `the state file at ${path} is unreadable: ${reason}; fix it or create it again with init`)
}

function errorCode(error: unknown): unknown {
  return typeof error === 'object' && error !== null && 'code' in error ? error.code : undefined
}

/** The page's "updated" time, in UTC to the minute. */
function stamp(now: Date): string {
  return `${now.toISOString().slice(0, 16).replace('T', ' ')} UTC`
}
