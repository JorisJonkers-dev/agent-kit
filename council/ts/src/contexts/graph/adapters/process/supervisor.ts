import { spawn as nodeSpawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { readFile } from 'node:fs/promises'

import { readDuBytes } from './disk-usage.js'
import {
  defaultProcessKiller,
  killDetachedProcessGroup,
} from './process-group-control.js'
import { ReattachedWorker, RunningWorker } from './session.js'
import { joinPrompt } from './session-support.js'
import type {
  SpawnInput,
  WorkerSupervisorDependencies,
  WorkerSupervisorEvent,
  WorkerSupervisorRuntimeEvent,
  WorkerSupervisorSnapshot,
  WorkerSupervisorSession,
  WorkerSupervisorStartRequest,
  WorkerSupervisorStatus,
} from './types.js'
import { optional } from './types.js'

export class WorkerSupervisorAdapter {
  private readonly spawnChild: NonNullable<WorkerSupervisorDependencies['spawn']>
  private readonly killPid: NonNullable<WorkerSupervisorDependencies['kill']>
  private readonly sleepFor: NonNullable<WorkerSupervisorDependencies['sleep']>
  private readonly nowMs: NonNullable<WorkerSupervisorDependencies['nowMs']>
  private readonly duBytes: NonNullable<WorkerSupervisorDependencies['duBytes']>
  private readonly onEvent: NonNullable<WorkerSupervisorDependencies['onEvent']>
  private readonly onSnapshot: NonNullable<WorkerSupervisorDependencies['onSnapshot']>
  private readonly pidAlive: NonNullable<WorkerSupervisorDependencies['isPidAlive']>
  private readonly readLog: NonNullable<WorkerSupervisorDependencies['readLogFile']>

  constructor(dependencies: WorkerSupervisorDependencies = {}) {
    this.spawnChild =
      dependencies.spawn ??
      ((command, args, options) => nodeSpawn(command, [...args], options))
    this.killPid = dependencies.kill ?? defaultProcessKiller
    this.sleepFor = dependencies.sleep ?? sleep
    this.nowMs = dependencies.nowMs ?? Date.now
    this.duBytes = dependencies.duBytes ?? readDuBytes
    this.onEvent = dependencies.onEvent ?? (() => undefined)
    this.onSnapshot = dependencies.onSnapshot ?? (() => undefined)
    this.pidAlive = dependencies.isPidAlive ?? defaultPidAlive
    this.readLog = dependencies.readLogFile ?? readFile
  }

  start(request: WorkerSupervisorStartRequest): WorkerSupervisorSession {
    return new RunningWorker(this, request)
  }

  reattach(
    request: WorkerSupervisorStartRequest,
    snapshot: WorkerSupervisorSnapshot,
  ): WorkerSupervisorSession {
    if (snapshot.task_id !== request.id) {
      throw new Error(`supervisor snapshot task_id must match request id: ${request.id}`)
    }
    const pid = snapshot.pid

    if (!isReattachableSnapshot(snapshot) || pid === undefined) {
      return terminalSnapshotSession(request, snapshot, 'stale-snapshot')
    }

    if (!this.pidAlive(pid)) {
      return terminalSnapshotSession(request, snapshot, 'dead-snapshot')
    }

    return new ReattachedWorker(this, request, { ...snapshot, pid })
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

  snapshot(snapshot: WorkerSupervisorSnapshot): void {
    void this.onSnapshot(snapshot)
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

  isPidAlive(pid: number): boolean {
    return this.pidAlive(pid)
  }

  readLogFile(path: string): Promise<Buffer> {
    return this.readLog(path)
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

function isReattachableSnapshot(snapshot: WorkerSupervisorSnapshot): boolean {
  return (
    snapshot.status === 'running' ||
    snapshot.status === 'detected' ||
    snapshot.status === 'restarting'
  )
}

function terminalSnapshotSession(
  request: WorkerSupervisorStartRequest,
  snapshot: WorkerSupervisorSnapshot,
  status: Extract<WorkerSupervisorStatus, 'dead-snapshot' | 'stale-snapshot'>,
): WorkerSupervisorSession {
  return {
    inject: () => Promise.reject(new Error(`worker ${request.id} has a terminal snapshot and cannot inject`)),
    result: Promise.resolve({
      exitCode: null,
      id: request.id,
      ...optional('modelTier', snapshot.model_tier),
      restarts: snapshot.restart_count,
      signal: null,
      status,
      stderr: '',
      stderrBytes: snapshot.offsets.stderr,
      stderrLogPath: snapshot.logs.stderr,
      stdout: '',
      stdoutBytes: snapshot.offsets.stdout,
      stdoutLogPath: snapshot.logs.stdout,
    }),
    stop: () => Promise.resolve(),
  }
}

function defaultPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    if (isNoSuchProcessError(error)) {
      return false
    }
    throw error
  }
}

function isNoSuchProcessError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { readonly code?: unknown }).code === 'ESRCH'
  )
}
