import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { planStateWrite, planTasksWrite, type RunStoreEvent } from '../../domain/runstore/index.js'
import type {
  Amendment,
  DesignLedger,
  ReviewVerdict,
  RoutingVerdict,
  RunState,
  Story,
  Task,
} from '../../domain/contracts/index.js'
import type { ClockPort, RunStorePort } from '../../ports/index.js'
import { AtomicWriter } from './atomic-writer.js'
import {
  DESIGN_LEDGER_FILE,
  RESULT_FILE,
  STORY_FILE,
  WORKERS_DIR,
  assertDesignLedger,
  assertPathSegment,
  assertRunState,
  assertStory,
  assertTasks,
  assertWorkerResult,
  parseJson,
  type WorkerResult,
} from './artifact-codec.js'
import { EventLog } from './event-log.js'

export interface FsRunStoreOptions {
  readonly clock?: ClockPort
  readonly lockRetryMs?: number
  readonly lockTimeoutMs?: number
  readonly tempId?: () => string
}

class DefaultClock implements ClockPort {
  now(): Date {
    return new Date()
  }

  monotonicMs(): number {
    return Number(process.hrtime.bigint() / 1_000_000n)
  }

  async sleep(ms: number): Promise<void> {
    await new Promise<void>((resolve) => {
      setTimeout(resolve, ms)
    })
  }
}

export class FsRunStoreAdapter implements RunStorePort {
  private readonly atomicWriter: AtomicWriter
  private readonly eventLog: EventLog
  private readonly root: string
  private readonly tempId: () => string
  private tempCounter = 0

  constructor(root: string, options: FsRunStoreOptions = {}) {
    const clock = options.clock ?? new DefaultClock()
    this.tempId =
      options.tempId ??
      (() =>
        `tmp-${String(clock.now().getTime())}-${String(process.pid)}-${String(++this.tempCounter)}`)
    this.atomicWriter = new AtomicWriter(root, this.tempId)
    this.eventLog = new EventLog(root, clock, options.lockRetryMs ?? 25, options.lockTimeoutMs ?? 30_000)
    this.root = root
  }

  async readState(runId: string): Promise<RunState> {
    return assertRunState(await this.readJson(runId, 'state.json'))
  }

  async writeState(runId: string, state: RunState): Promise<void> {
    assertRunState(state)
    await this.atomicWriter.executeJsonPlan(planStateWrite(runId, state, this.nextTempId()))
  }

  async readTasks(runId: string): Promise<readonly Task[]> {
    return assertTasks(await this.readJson(runId, 'tasks.json'))
  }

  async writeTasks(runId: string, tasks: readonly Task[]): Promise<void> {
    assertTasks(tasks)
    await this.atomicWriter.executeJsonPlan(planTasksWrite(runId, tasks, this.nextTempId()))
  }

  async readStory(runId: string): Promise<Story> {
    return assertStory(await this.readJson(runId, STORY_FILE))
  }

  async writeStory(runId: string, story: Story): Promise<void> {
    assertStory(story)
    await this.atomicWriter.writeJson(this.runFile(runId, STORY_FILE), story)
  }

  async readDesignLedger(runId: string): Promise<DesignLedger> {
    return assertDesignLedger(await this.readJson(runId, DESIGN_LEDGER_FILE))
  }

  async writeDesignLedger(runId: string, ledger: DesignLedger): Promise<void> {
    assertDesignLedger(ledger)
    await this.atomicWriter.writeJson(this.runFile(runId, DESIGN_LEDGER_FILE), ledger)
  }

  async appendReviewVerdict(runId: string, verdict: ReviewVerdict): Promise<void> {
    await this.eventLog.appendReviewVerdict(runId, verdict)
  }

  async appendRoutingVerdict(runId: string, verdict: RoutingVerdict): Promise<void> {
    await this.eventLog.appendRoutingVerdict(runId, verdict)
  }

  async appendAmendment(runId: string, amendment: Amendment): Promise<void> {
    await this.eventLog.appendAmendment(runId, amendment)
  }

  async readEvents(runId: string): Promise<readonly RunStoreEvent[]> {
    return this.eventLog.read(runId, this.eventLog.eventPath(runId))
  }

  async readWorkerResult(runId: string, taskId: string): Promise<WorkerResult> {
    return assertWorkerResult(await this.readJson(runId, WORKERS_DIR, taskId, RESULT_FILE), taskId)
  }

  async writeWorkerResult(runId: string, taskId: string, result: WorkerResult): Promise<void> {
    assertPathSegment('taskId', taskId)
    assertWorkerResult(result, taskId)
    await this.atomicWriter.writeJson(this.runFile(runId, WORKERS_DIR, taskId, RESULT_FILE), result)
  }

  private nextTempId(): string {
    return this.tempId()
  }

  private async readJson(runId: string, ...pathSegments: readonly string[]): Promise<unknown> {
    assertPathSegment('runId', runId)
    pathSegments.forEach((segment) => {
      if (segment !== WORKERS_DIR) assertPathSegment('path segment', segment)
    })
    return parseJson(await readFile(this.runFile(runId, ...pathSegments), 'utf8'))
  }

  private runFile(runId: string, ...pathSegments: readonly string[]): string {
    assertPathSegment('runId', runId)
    return join(this.root, runId, ...pathSegments)
  }
}
