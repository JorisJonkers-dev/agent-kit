import { execFile } from 'node:child_process'
import { spawn as nodeSpawn } from 'node:child_process'
import type { ChildProcess, SpawnOptions } from 'node:child_process'
import { promisify } from 'node:util'

import {
  advanceEscalation,
  appendLoopLine,
  createEscalationState,
  createLoopDetectorState,
  createStallDetectorState,
  evaluateDiskUsageCap,
  evaluateStall,
} from '../../domain/watchdog/index.js'
import type {
  DiskUsageCapDetection,
  EscalationState,
  LoopDetection,
  LoopDetectorConfig,
  LoopDetectorState,
  StallDetection,
  StallDetectorState,
} from '../../domain/watchdog/index.js'

const execFileAsync = promisify(execFile)

export type WorkerSupervisorStatus = 'completed' | 'failed' | 'stopped' | 'stalled' | 'disk-cap'
export type WorkerSupervisorDetection = StallDetection | LoopDetection | DiskUsageCapDetection
export type InjectDeliveryMode = 'streaming-stdin' | 'checkpoint-and-resume'

export interface WorkerSupervisorWatchdogConfig {
  readonly stallAfterS?: number
  readonly windowSize?: number
  readonly repeatLimit?: number
  readonly maxCycleGram?: number
  readonly maxRestarts?: number
  readonly enableTierEscalation?: boolean
  readonly diskCapBytes?: number
}

export interface WorkerSupervisorStartRequest {
  readonly id: string
  readonly command: string
  readonly args?: readonly string[]
  readonly worktree: string
  readonly env?: Readonly<Record<string, string>>
  readonly stdin?: string
  readonly restartPreamble?: string
  readonly checkpointPreamble?: string
  readonly supportsStreamingStdin?: boolean
  readonly modelTier?: string
  readonly escalationModelTier?: string
  readonly pollIntervalMs?: number
  readonly killGraceMs?: number
  readonly watchdog?: WorkerSupervisorWatchdogConfig
}

export interface WorkerSupervisorResult {
  readonly id: string
  readonly status: WorkerSupervisorStatus
  readonly exitCode: number | null
  readonly signal: NodeJS.Signals | null
  readonly stdout: string
  readonly stderr: string
  readonly restarts: number
  readonly modelTier?: string
  readonly detection?: WorkerSupervisorDetection
}

export interface InjectDelivery {
  readonly mode: InjectDeliveryMode
  readonly restarted: boolean
}

export interface WorkerSupervisorSession {
  readonly result: Promise<WorkerSupervisorResult>
  inject(turn: string): Promise<InjectDelivery>
  stop(reason?: string): Promise<void>
}

export type WorkerSupervisorEvent =
  | { readonly type: 'started'; readonly pid: number | undefined; readonly restart: number }
  | { readonly type: 'stdout'; readonly chunk: string }
  | { readonly type: 'stderr'; readonly chunk: string }
  | { readonly type: 'detected'; readonly detection: WorkerSupervisorDetection }
  | { readonly type: 'terminated'; readonly signal: NodeJS.Signals; readonly pid: number | undefined }
  | { readonly type: 'restarted'; readonly restart: number; readonly preamble: string }
  | { readonly type: 'tier-escalated'; readonly modelTier: string }
  | { readonly type: 'injected'; readonly mode: InjectDeliveryMode }
  | { readonly type: 'exited'; readonly exitCode: number | null; readonly signal: NodeJS.Signals | null }
  | { readonly type: 'stopped'; readonly reason: string }

export interface WorkerSupervisorDependencies {
  readonly spawn?: (
    command: string,
    args: readonly string[],
    options: SpawnOptions,
  ) => ChildProcess
  readonly kill?: (pid: number, signal: NodeJS.Signals) => void
  readonly sleep?: (ms: number) => Promise<void>
  readonly nowMs?: () => number
  readonly duBytes?: (path: string) => Promise<number>
  readonly onEvent?: (event: WorkerSupervisorEvent) => void
}

interface ActiveProcess {
  readonly child: ChildProcess
  exited: boolean
  exitCode: number | null
  signal: NodeJS.Signals | null
  terminating: boolean
  exitedPromise: Promise<void>
}

interface SpawnInput {
  readonly preamble?: string
  readonly modelTier?: string
}

interface WatchdogConfig {
  readonly stallAfterS: number
  readonly loop: LoopDetectorConfig
  readonly maxRestarts: number
  readonly enableTierEscalation: boolean
  readonly diskCapBytes?: number
}

type MaybePromise<T> = T | Promise<T>

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
    this.killPid = dependencies.kill ?? ((pid, signal) => process.kill(pid, signal))
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
      ...(input.modelTier === undefined ? {} : { COUNCIL_MODEL_TIER: input.modelTier }),
    }
    const child = this.spawnChild(request.command, request.args ?? [], {
      cwd: request.worktree,
      detached: true,
      env,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    this.onEvent({ pid: child.pid, restart: run, type: 'started' })
    writeInitialStdin(child, joinPrompt(input.preamble, request.stdin))
    return child
  }

  emit(event: WorkerSupervisorEvent): void {
    this.onEvent(event)
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
    this.killPid(-pid, signal)
  }
}

class RunningWorker implements WorkerSupervisorSession {
  readonly result: Promise<WorkerSupervisorResult>
  private readonly supervisor: WorkerSupervisorAdapter
  private readonly request: WorkerSupervisorStartRequest
  private readonly watchdog: WatchdogConfig
  private readonly pollIntervalMs: number
  private readonly killGraceMs: number
  private active: ActiveProcess
  private run = 0
  private stdout = ''
  private stderr = ''
  private logBytes = 0
  private restarts = 0
  private modelTier: string | undefined
  private stallState: StallDetectorState
  private loopState: LoopDetectorState = createLoopDetectorState()
  private escalationState: EscalationState = createEscalationState()
  private pendingLoopDetection: LoopDetection | null = null
  private handlingDetection = false
  private finished = false
  private resolveResult!: (result: WorkerSupervisorResult) => void

  constructor(supervisor: WorkerSupervisorAdapter, request: WorkerSupervisorStartRequest) {
    this.supervisor = supervisor
    this.request = request
    this.watchdog = normalizeWatchdogConfig(request.watchdog)
    this.pollIntervalMs = request.pollIntervalMs ?? 15_000
    this.killGraceMs = request.killGraceMs ?? 5_000
    this.modelTier = request.modelTier
    this.stallState = createStallDetectorState(supervisor.now())
    this.result = new Promise<WorkerSupervisorResult>((resolve) => {
      this.resolveResult = resolve
    })
    this.active = this.spawn({ ...optional('modelTier', this.modelTier) })
    void this.poll()
  }

  async inject(turn: string): Promise<InjectDelivery> {
    if (this.finished) {
      throw new Error(`worker ${this.request.id} has already finished`)
    }

    if (this.request.supportsStreamingStdin === true && this.active.child.stdin?.writable === true) {
      this.active.child.stdin.write(`${turn}\n`)
      this.supervisor.emit({ mode: 'streaming-stdin', type: 'injected' })
      return { mode: 'streaming-stdin', restarted: false }
    }

    const preamble = joinPrompt(this.request.checkpointPreamble ?? 'Resume from checkpoint.', turn) ?? ''
    await this.restartWithPreamble(preamble, this.modelTier)
    this.supervisor.emit({ mode: 'checkpoint-and-resume', type: 'injected' })
    return { mode: 'checkpoint-and-resume', restarted: true }
  }

  async stop(reason = 'requested'): Promise<void> {
    if (!this.finished) {
      await this.terminate(this.active, 'SIGTERM')
      this.finish('stopped', null, null, undefined)
      this.supervisor.emit({ reason, type: 'stopped' })
    }
  }

  private async poll(): Promise<void> {
    while (!this.finished) {
      await this.supervisor.sleep(this.pollIntervalMs)
      if (this.isFinished() || this.active.exited) {
        return
      }

      const maybeDetection = this.pollDetection()
      const detection = isPromiseLike(maybeDetection) ? await maybeDetection : maybeDetection
      if (detection !== null) {
        const handled = this.handleDetection(detection)
        if (isPromiseLike(handled)) {
          await handled
        }
      }
    }
  }

  private pollDetection(): MaybePromise<WorkerSupervisorDetection | null> {
    if (this.watchdog.diskCapBytes !== undefined) {
      return this.supervisor.diskUsageBytes(this.request.worktree).then((duBytes) => {
        const diskDetection = evaluateDiskUsageCap({
          capBytes: this.watchdog.diskCapBytes ?? 0,
          duBytes,
        })
        return diskDetection ?? this.pollLogDetection()
      })
    }

    return this.pollLogDetection()
  }

  private pollLogDetection(): WorkerSupervisorDetection | null {
    if (this.pendingLoopDetection !== null) {
      const detection = this.pendingLoopDetection
      this.pendingLoopDetection = null
      return detection
    }

    const stall = evaluateStall(this.stallState, {
      logBytes: this.logBytes,
      nowMs: this.supervisor.now(),
      stallAfterS: this.watchdog.stallAfterS,
    })
    this.stallState = stall.state
    return stall.detection
  }

  private handleDetection(detection: WorkerSupervisorDetection): MaybePromise<void> {
    if (!this.handlingDetection && !this.finished) {
      this.handlingDetection = true
      this.supervisor.emit({ detection, type: 'detected' })

      const handled =
        detection.kind === 'disk-cap'
          ? thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
              this.finish('disk-cap', null, null, detection)
            })
          : this.advanceRestartPolicy(detection)

      if (isPromiseLike(handled)) {
        return handled.finally(() => {
          this.handlingDetection = false
        })
      }

      this.handlingDetection = false
    }
  }

  private advanceRestartPolicy(detection: WorkerSupervisorDetection): MaybePromise<void> {
    const first = advanceEscalation(this.escalationState, {
      enableTierEscalation: this.watchdog.enableTierEscalation,
    })
    this.escalationState = first.state

    if (first.action === 'terminate') {
      return thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
        const next = advanceEscalation(this.escalationState, {
          enableTierEscalation: this.watchdog.enableTierEscalation,
        })
        this.escalationState = next.state
        return this.applyEscalationAction(next.action, detection)
      })
    }

    return this.applyEscalationAction(first.action, detection)
  }

  private applyEscalationAction(
    action: ReturnType<typeof advanceEscalation>['action'],
    detection: WorkerSupervisorDetection,
  ): MaybePromise<void> {
    if (action === 'retry-with-preamble') {
      return this.restartWithPreamble(this.restartPreamble(detection), this.modelTier, detection)
    }

    if (action === 'escalate-tier' && this.request.escalationModelTier !== undefined) {
      this.modelTier = this.request.escalationModelTier
      this.supervisor.emit({ modelTier: this.modelTier, type: 'tier-escalated' })
      return this.restartWithPreamble(this.restartPreamble(detection), this.modelTier, detection)
    }

    return thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
      this.finish('stalled', null, null, detection)
    })
  }

  private restartWithPreamble(
    preamble: string,
    modelTier: string | undefined,
    detection?: WorkerSupervisorDetection,
  ): MaybePromise<void> {
    return thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
      if (this.restarts >= this.watchdog.maxRestarts) {
        this.finish('stalled', null, null, detection)
        return
      }

      this.restarts += 1
      this.pendingLoopDetection = null
      this.stallState = createStallDetectorState(this.supervisor.now(), this.logBytes)
      this.active = this.spawn(spawnInput(preamble, modelTier))
      this.supervisor.emit({ preamble, restart: this.restarts, type: 'restarted' })
    })
  }

  private spawn(input: SpawnInput): ActiveProcess {
    this.run += 1
    const run = this.run
    const child = this.supervisor.createChild(this.request, run, input)
    const active: ActiveProcess = {
      child,
      exitCode: null,
      exited: false,
      exitedPromise: Promise.resolve(),
      signal: null,
      terminating: false,
    }
    active.exitedPromise = new Promise<void>((resolve) => {
      child.once('exit', (exitCode: number | null, signal: NodeJS.Signals | null) => {
        active.exited = true
        active.exitCode = exitCode
        active.signal = signal
        this.supervisor.emit({ exitCode, signal, type: 'exited' })
        if (this.active === active && !this.finished && !active.terminating) {
          this.finish(exitCode === 0 ? 'completed' : 'failed', exitCode, signal, undefined)
        }
        resolve()
      })
    })
    child.stdout?.on('data', (chunk: Buffer | string) => { this.recordOutput('stdout', chunk); })
    child.stderr?.on('data', (chunk: Buffer | string) => { this.recordOutput('stderr', chunk); })
    return active
  }

  private recordOutput(stream: 'stdout' | 'stderr', chunk: Buffer | string): void {
    const text = chunk.toString()
    this.logBytes += Buffer.byteLength(text)
    if (stream === 'stdout') {
      this.stdout += text
      this.supervisor.emit({ chunk: text, type: 'stdout' })
    } else {
      this.stderr += text
      this.supervisor.emit({ chunk: text, type: 'stderr' })
    }

    for (const line of text.split(/\r?\n/u)) {
      const loop = appendLoopLine(this.loopState, line, this.watchdog.loop)
      this.loopState = loop.state
      this.pendingLoopDetection = this.pendingLoopDetection ?? loop.detection
    }
  }

  private terminate(active: ActiveProcess, signal: NodeJS.Signals): MaybePromise<void> {
    if (active.exited || active.child.pid === undefined) {
      return
    }

    active.terminating = true
    try {
      if (!this.killProcessGroup(active.child.pid, signal)) {
        active.exited = true
        return
      }

      this.supervisor.emit({ pid: active.child.pid, signal, type: 'terminated' })
      if (this.hasExited(active)) {
        return
      }

      const waited = this.waitForExitOrKill(active)
      if (isPromiseLike(waited)) {
        return waited.catch((error: unknown) => {
          active.terminating = false
          throw error
        })
      }
    } catch (error) {
      active.terminating = false
      throw error
    }
  }

  private waitForExitOrKill(active: ActiveProcess): MaybePromise<void> {
    return Promise.race([active.exitedPromise, this.supervisor.sleep(this.killGraceMs)]).then(
      () => {
        if (!active.exited && active.child.pid !== undefined) {
          if (!this.killProcessGroup(active.child.pid, 'SIGKILL')) {
            active.exited = true
            return
          }

          this.supervisor.emit({ pid: active.child.pid, signal: 'SIGKILL', type: 'terminated' })
          return this.hasExited(active) ? undefined : active.exitedPromise
        }
      },
    )
  }

  private killProcessGroup(pid: number, signal: NodeJS.Signals): boolean {
    try {
      this.supervisor.killGroup(pid, signal)
      return true
    } catch (error) {
      if (!isNoSuchProcessError(error)) {
        throw error
      }
      return false
    }
  }

  private isFinished(): boolean {
    return this.finished
  }

  private hasExited(active: ActiveProcess): boolean {
    return active.exited
  }

  private restartPreamble(detection: WorkerSupervisorDetection): string {
    return joinPrompt(
      this.request.restartPreamble ?? 'Previous attempt was interrupted by the watchdog.',
      `Watchdog detection: ${detection.kind}.`,
    ) ?? ''
  }

  private finish(
    status: WorkerSupervisorStatus,
    exitCode: number | null,
    signal: NodeJS.Signals | null,
    detection: WorkerSupervisorDetection | undefined,
  ): void {
    if (!this.finished) {
      this.finished = true
      this.resolveResult({
        ...optional('detection', detection),
        exitCode,
        id: this.request.id,
        ...optional('modelTier', this.modelTier),
        restarts: this.restarts,
        signal,
        status,
        stderr: this.stderr,
        stdout: this.stdout,
      })
    }
  }
}

export async function readDuBytes(path: string): Promise<number> {
  const { stdout } = await execFileAsync('du', ['-sk', path])
  return parseDuBytes(path, stdout)
}

export function parseDuBytes(path: string, stdout: string): number {
  const [kilobytes] = stdout.trim().split(/\s+/u)
  const parsed = Number(kilobytes)

  if (!Number.isFinite(parsed)) {
    throw new Error(`could not parse du output for ${path}: ${stdout}`)
  }

  return parsed * 1024
}

function normalizeWatchdogConfig(config: WorkerSupervisorWatchdogConfig = {}): WatchdogConfig {
  return {
    ...optional('diskCapBytes', config.diskCapBytes),
    enableTierEscalation: config.enableTierEscalation ?? true,
    loop: {
      ...optional('maxCycleGram', config.maxCycleGram),
      repeatLimit: config.repeatLimit ?? 3,
      windowSize: config.windowSize ?? 20,
    },
    maxRestarts: config.maxRestarts ?? 2,
    stallAfterS: config.stallAfterS ?? 300,
  }
}

function writeInitialStdin(child: ChildProcess, input: string | undefined): void {
  if (input === undefined || child.stdin === null) {
    return
  }

  child.stdin.write(input.endsWith('\n') ? input : `${input}\n`)
}

function joinPrompt(...parts: readonly (string | undefined)[]): string | undefined {
  const joined = parts
    .filter((part): part is string => part !== undefined && part.length > 0)
    .join('\n\n')
  return joined.length === 0 ? undefined : joined
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    timer.unref()
  })
}

function isNoSuchProcessError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { readonly code?: unknown }).code === 'ESRCH'
  )
}

function spawnInput(preamble: string, modelTier: string | undefined): SpawnInput {
  return {
    preamble,
    ...optional('modelTier', modelTier),
  }
}

function isPromiseLike<T>(value: MaybePromise<T>): value is Promise<T> {
  return (
    typeof value === 'object' &&
    value !== null &&
    'then' in value &&
    typeof (value as { readonly then?: unknown }).then === 'function'
  )
}

function thenMaybe<T, Next>(
  value: MaybePromise<T>,
  next: () => MaybePromise<Next>,
): MaybePromise<Next> {
  return isPromiseLike(value) ? value.then(next) : next()
}

function optional<Key extends string, Value>(
  key: Key,
  value: Value | undefined,
): Partial<Record<Key, NonNullable<Value>>> {
  return (value === undefined ? {} : { [key]: value }) as Partial<Record<Key, NonNullable<Value>>>
}
