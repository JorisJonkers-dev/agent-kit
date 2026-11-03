import { applyAssignments, landedAssignments, parseAssignments } from '../domain/assignments.js'
import { buildBoard, type Shown } from '../domain/board.js'
import { renderPage } from '../domain/page.js'
import { parseMilestones, seedState, syncState } from '../domain/seed.js'
import { specText, stateBase, stateLocation, type StateLocation } from '../domain/spec.js'
import { BuildTrackerError, parseState, type JsonObject, type TrackerState } from '../domain/state.js'
import type { BuildTrackerClockPort, BuildTrackerFilesPort, BuildTrackerIssuesPort, BuildTrackerRepositoryPort } from './ports.js'
import { resolveSpec, type SpecSource } from './specs.js'

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

export interface InitInput {
  /** The spec as typed: an issue reference or a local markdown file. */
  readonly spec: string
  /** Each `<name>=<ticket>`, in order. */
  readonly milestones: readonly string[]
}

/** Where a spec's state file and page live. */
export interface PathResult {
  readonly status: 'path'
  readonly spec: string
  readonly state: string
  readonly page: string
}

export interface InitResult extends Omit<PathResult, 'status'> {
  readonly status: 'initialized'
  readonly tickets: number
  readonly milestones: readonly string[]
}

export interface SyncResult extends Omit<PathResult, 'status'> {
  readonly status: 'synced'
  readonly tickets: number
  /** Tickets the state did not have before. */
  readonly added: readonly number[]
  /** Tickets marked merged because their issue closed. */
  readonly merged: readonly number[]
}

export interface RefreshInput {
  /** The spec as typed: an issue reference or a local markdown file. */
  readonly spec: string
  /** The tickets the flow records as landed, each `12`, `#12` or a local ticket's `01`. */
  readonly landed: readonly string[]
}

export interface RefreshResult extends Omit<PathResult, 'status'> {
  readonly status: 'refreshed'
  /** `init` when the spec had no state, `sync` when it had one. */
  readonly ran: 'init' | 'sync'
  readonly tickets: number
  /** Tickets the state did not have before; every ticket on `init`. */
  readonly added: readonly number[]
  /** Tickets that are merged now and were not before: newly closed, or named as landed. */
  readonly merged: readonly number[]
  readonly counts: RenderResult['counts']
}

export interface BuildTrackerCommands {
  readonly render: { run(input: RenderInput): Promise<RenderResult> }
  readonly set: { run(input: SetInput): Promise<SetResult> }
  readonly init: { run(input: InitInput): Promise<InitResult> }
  readonly sync: { run(input: { readonly spec: string }): Promise<SyncResult> }
  readonly path: { run(input: { readonly spec: string }): Promise<PathResult> }
  readonly refresh: { run(input: RefreshInput): Promise<RefreshResult> }
}

export interface BuildTrackerDependencies {
  readonly files: BuildTrackerFilesPort
  readonly clock: BuildTrackerClockPort
  readonly issues: BuildTrackerIssuesPort
  readonly repositories: BuildTrackerRepositoryPort
  /** The directory a relative spec path and a bare issue number are read against. */
  readonly cwd: string
  /** The environment the state home comes from: XDG_STATE_HOME, else HOME. */
  readonly env: Readonly<Record<string, string | undefined>>
}

export function createBuildTrackerCommands(deps: BuildTrackerDependencies): BuildTrackerCommands {
  const locate = async (argument: string): Promise<{ source: SpecSource; location: StateLocation; spec: string }> => {
    const source = await resolveSpec(argument, deps)
    const base = stateBase(deps.env)
    if (base === undefined) {
      throw new BuildTrackerError('no-state-home', 'neither XDG_STATE_HOME nor HOME is an absolute path, so there is no place outside the repository for the state; set one of them')
    }
    return { location: stateLocation(base, source.spec), source, spec: specText(source.spec) }
  }
  return {
    init: {
      async run(input) {
        const { location, source, spec } = await locate(input.spec)
        if (await exists(deps.files, location.state)) {
          throw new BuildTrackerError('state-exists', `a state for ${spec} already exists at ${location.state}; bring it up to date with sync`)
        }
        const read = await source.read()
        const milestones = parseMilestones(input.milestones, read)
        await writeState(deps.files, location.state, seedState(source.spec, read, milestones))
        return {
          milestones: milestones.map((milestone) => milestone.name),
          page: location.page,
          spec,
          state: location.state,
          status: 'initialized',
          tickets: read.tickets.length,
        }
      },
    },
    path: {
      async run(input) {
        const { location, spec } = await locate(input.spec)
        return { page: location.page, spec, state: location.state, status: 'path' }
      },
    },
    refresh: {
      async run(input) {
        const { location, source, spec } = await locate(input.spec)
        const before = (await exists(deps.files, location.state)) ? await loadState(deps.files, location.state) : undefined
        const read = await source.read()
        const outcome =
          before === undefined
            ? { added: read.tickets.map((ticket) => ticket.number), raw: seedState(source.spec, read, []) }
            : syncState(before.raw, read)
        const raw = applyAssignments(outcome.raw, landedAssignments(input.landed, parseState(outcome.raw)))
        const state = parseState(raw)
        const wasMerged = new Set(before?.state.tickets.filter((ticket) => ticket.status === 'merged').map((ticket) => ticket.number))
        // The page first: when it cannot be written, the state stays as it was.
        await deps.files.writeAtomic(location.page, renderPage(state, stamp(deps.clock.now())))
        await writeState(deps.files, location.state, raw)
        return {
          added: outcome.added,
          counts: buildBoard(state).counts,
          merged: state.tickets.filter((ticket) => ticket.status === 'merged' && !wasMerged.has(ticket.number)).map((ticket) => ticket.number),
          page: location.page,
          ran: before === undefined ? 'init' : 'sync',
          spec,
          state: location.state,
          status: 'refreshed',
          tickets: state.tickets.length,
        }
      },
    },
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
        await writeState(deps.files, input.state, applyAssignments(raw, assignments))
        return {
          changed: assignments.map((assignment) => ({ status: assignment.status, ticket: assignment.ticket })),
          status: 'set',
        }
      },
    },
    sync: {
      async run(input) {
        const { location, source, spec } = await locate(input.spec)
        const { raw } = await loadState(deps.files, location.state)
        const outcome = syncState(raw, await source.read())
        const synced = parseState(outcome.raw)
        await writeState(deps.files, location.state, outcome.raw)
        return {
          added: outcome.added,
          merged: outcome.merged,
          page: location.page,
          spec,
          state: location.state,
          status: 'synced',
          tickets: synced.tickets.length,
        }
      },
    },
  }
}

async function writeState(files: BuildTrackerFilesPort, path: string, raw: JsonObject): Promise<void> {
  await files.writeAtomic(path, `${JSON.stringify(raw, null, 2)}\n`)
}

async function exists(files: BuildTrackerFilesPort, path: string): Promise<boolean> {
  try {
    await files.readText(path)
    return true
  } catch (error) {
    if (errorCode(error) === 'ENOENT') return false
    throw unreadable(path, error instanceof Error ? error.message : String(error))
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
