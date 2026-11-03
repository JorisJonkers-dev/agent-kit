import type { Counts } from '../domain/board.js'
import { renderPage } from '../domain/page.js'
import type { TicketFiles } from '../domain/spec.js'
import { BuildTrackerError, parseState, type BuildTrackerState, type JsonObject } from '../domain/state.js'
import type { BuildTrackerClockPort, BuildTrackerFilesPort } from './ports.js'

/** A state file as read: its parsed JSON, kept whole for writing back, and the state it holds. */
export interface LoadedState {
  readonly raw: JsonObject
  readonly state: BuildTrackerState
}

/**
 * The state file read and checked, or undefined when there is none. Any
 * other failure is `unreadable-state`, which names the file and says to
 * delete it and run init again.
 */
export async function readStateFile(files: BuildTrackerFilesPort, path: string): Promise<LoadedState | undefined> {
  let text: string
  try {
    text = await files.readText(path)
  } catch (error) {
    if (errorCode(error) === 'ENOENT') return undefined
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

/** The state file read and checked; a missing one is `missing-state`, which names init. */
export async function loadState(files: BuildTrackerFilesPort, path: string): Promise<LoadedState> {
  const loaded = await readStateFile(files, path)
  if (loaded === undefined) throw new BuildTrackerError('missing-state', `there is no state file at ${path}; create it with init`)
  return loaded
}

export async function writeState(files: BuildTrackerFilesPort, path: string, raw: JsonObject): Promise<void> {
  await files.writeAtomic(path, `${JSON.stringify(raw, null, 2)}\n`)
}

/** Renders the state's page to `path`, stamped with the clock's time, and returns its count per shown status. */
export async function writePage(
  deps: { readonly files: BuildTrackerFilesPort; readonly clock: BuildTrackerClockPort },
  path: string,
  state: BuildTrackerState,
  ticketFiles: TicketFiles | undefined,
): Promise<Counts> {
  const page = renderPage(state, stamp(deps.clock.now()), ticketFiles)
  await deps.files.writeAtomic(path, page.html)
  return page.counts
}

function unreadable(path: string, reason: string): BuildTrackerError {
  return new BuildTrackerError('unreadable-state', `the state file at ${path} is unreadable: ${reason}; delete it and run init again`)
}

function errorCode(error: unknown): unknown {
  return typeof error === 'object' && error !== null && 'code' in error ? error.code : undefined
}

/** The page's "updated" time, in UTC to the minute. */
function stamp(now: Date): string {
  return `${now.toISOString().slice(0, 16).replace('T', ' ')} UTC`
}
