import { planWaves } from '../contexts/graph/index.js'
import type { LiveRunArtifacts, LiveRunDirReaderPort, WorkerResult } from '../ports/index.js'

import { renderRunStatusJson, renderRunStatusTable } from './status-render.js'
import { projectRunView, type RunSummary, type RunView, type RunViewClock } from './status.js'

export const DEFAULT_STATUS_WATCH_INTERVAL_MS = 1_000

export type StatusWatchFrameFormat = 'json' | 'table'

export interface StatusWatchInput {
  readonly color?: boolean
  readonly intervalMs?: number
  readonly json?: boolean
  readonly once?: boolean
  readonly runDir: string
  readonly suppressUnchanged?: boolean
}

export interface StatusWatchTickerInput {
  readonly intervalMs: number
}

export interface StatusWatchTickerPort {
  ticks(input: StatusWatchTickerInput): AsyncIterable<unknown>
}

export interface StatusWatchWorkflowDeps {
  readonly clock: RunViewClock
  readonly readRunDir: LiveRunDirReaderPort['readRunDir']
  readonly ticker?: StatusWatchTickerPort
}

export interface StatusWatchFrame {
  readonly format: StatusWatchFrameFormat
  readonly output: string
  readonly sequence: number
}

export async function* statusWatchWorkflow(
  input: StatusWatchInput,
  deps: StatusWatchWorkflowDeps,
): AsyncIterable<StatusWatchFrame> {
  const ticker = statusWatchTicker(input, deps)
  let previousOutput: string | undefined
  let sequence = 0

  const firstFrame = await renderStatusFrame(input, deps.clock, deps.readRunDir, sequence)
  yield firstFrame
  previousOutput = firstFrame.output
  sequence += 1

  if (ticker === undefined) return

  for await (const tick of ticker.ticks({ intervalMs: input.intervalMs ?? DEFAULT_STATUS_WATCH_INTERVAL_MS })) {
    void tick
    const frame = await renderStatusFrame(input, deps.clock, deps.readRunDir, sequence)
    if (input.suppressUnchanged === true && frame.output === previousOutput) {
      continue
    }
    yield frame
    previousOutput = frame.output
    sequence += 1
  }
}

function statusWatchTicker(
  input: StatusWatchInput,
  deps: StatusWatchWorkflowDeps,
): StatusWatchTickerPort | undefined {
  if (input.json === true || input.once === true) return undefined
  if (deps.ticker === undefined) throw new Error('ticker dependency is required for status watch mode')
  return deps.ticker
}

async function renderStatusFrame(
  input: StatusWatchInput,
  clock: RunViewClock,
  readRunDir: LiveRunDirReaderPort['readRunDir'],
  sequence: number,
): Promise<StatusWatchFrame> {
  const artifacts = await readRunDir(input.runDir)
  const view = projectRunView({
    clock,
    events: artifacts.events,
    summary: runSummaryFromArtifacts(artifacts),
    supervisorSnapshots: [...artifacts.workerSupervisorSnapshots.values()],
  })
  const format = statusWatchFrameFormat(input)
  return {
    format,
    output: `${renderView(view, format, input)}\n`,
    sequence,
  }
}

function runSummaryFromArtifacts(artifacts: LiveRunArtifacts): RunSummary {
  const workerResults = workerResultsFromArtifacts(artifacts)
  return {
    ...(artifacts.normalized.report === undefined ? {} : { report: artifacts.normalized.report }),
    run: artifacts.normalized.runId,
    state: artifacts.normalized.state,
    tasks: artifacts.normalized.tasks,
    waves: artifacts.normalized.report?.waves ?? planWaves(artifacts.normalized.tasks),
    workerResults,
  }
}

function workerResultsFromArtifacts(artifacts: LiveRunArtifacts): readonly WorkerResult[] {
  const merged = new Map(artifacts.normalized.workerResults)
  for (const [taskId, result] of artifacts.workerResults) {
    merged.set(taskId, result)
  }
  return [...merged.values()]
}

function statusWatchFrameFormat(input: StatusWatchInput): StatusWatchFrameFormat {
  return input.json === true ? 'json' : 'table'
}

function renderView(view: RunView, format: StatusWatchFrameFormat, input: StatusWatchInput): string {
  return format === 'json' ? renderRunStatusJson(view) : renderRunStatusTable(view, { color: input.color === true })
}
