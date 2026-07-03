import { spawn as nodeSpawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'

import { readDuBytes } from './disk-usage.js'
import {
  defaultProcessKiller,
  killDetachedProcessGroup,
} from './process-group-control.js'
import { RunningWorker } from './session.js'
import { joinPrompt } from './session-support.js'
import type {
  SpawnInput,
  WorkerSupervisorDependencies,
  WorkerSupervisorEvent,
  WorkerSupervisorRuntimeEvent,
  WorkerSupervisorSession,
  WorkerSupervisorStartRequest,
} from './types.js'
import { optional } from './types.js'

export class WorkerSupervisorAdapter {
  private readonly spawnChild: NonNullable<WorkerSupervisorDependencies['spawn']>
  private readonly killPid: NonNullable<WorkerSupervisorDependencies['kill']>
  private readonly sleepFor: NonNullable<WorkerSupervisorDependencies['sleep']>
  private readonly nowMs: NonNullable<WorkerSupervisorDependencies['nowMs']>
  private readonly duBytes: NonNullable<WorkerSupervisorDependencies['duBytes']>
  private readonly onEvent: NonNullable<WorkerSupervisorDependencies['onEvent']>

  constructor(dependencies: WorkerSupervisorDependencies = {}) {
    this.spawnChild =
      dependencies.spawn ??
      ((command, args, options) => nodeSpawn(command, [...args], options))
    this.killPid = dependencies.kill ?? defaultProcessKiller
    this.sleepFor = dependencies.sleep ?? sleep
    this.nowMs = dependencies.nowMs ?? Date.now
    this.duBytes = dependencies.duBytes ?? readDuBytes
    this.onEvent = dependencies.onEvent ?? (() => undefined)
  }

  start(request: WorkerSupervisorStartRequest): WorkerSupervisorSession {
    return new RunningWorker(this, request)
  }

  createChild(request: WorkerSupervisorStartRequest, run: number, input: SpawnInput): ChildProcess {
    const env = {
      ...process.env,
      ...request.env,
      KB_AUTO_MCP_DISABLED: '1',
      ...(input.modelTier === undefined ? {} : { COUNCIL_MODEL_TIER: input.modelTier }),
    }
    const child = this.spawnChild(request.command, request.args ?? [], {
      cwd: request.worktree,
      detached: true,
      env,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    this.onEvent({
      attemptId: run,
      ...optional('detection', input.detection),
      ...optional('modelTier', input.modelTier),
      ...optional('pid', child.pid),
      restart: run,
      restartCount: run - 1,
      taskId: request.id,
      type: 'started',
    })
    writeInitialStdin(child, joinPrompt(input.preamble, request.stdin))
    return child
  }

  emit(event: WorkerSupervisorRuntimeEvent): void {
    this.onEvent(event as WorkerSupervisorEvent)
  }

  now(): number {
    return this.nowMs()
  }

  sleep(ms: number): Promise<void> {
    return this.sleepFor(ms)
  }

  diskUsageBytes(path: string): Promise<number> {
    return this.duBytes(path)
  }

  killGroup(pid: number, signal: NodeJS.Signals): void {
    killDetachedProcessGroup(pid, signal, this.killPid)
  }
}

function writeInitialStdin(child: ChildProcess, input: string | undefined): void {
  if (input === undefined || child.stdin === null) {
    return
  }

  child.stdin.write(input.endsWith('\n') ? input : `${input}\n`)
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    timer.unref()
  })
}
