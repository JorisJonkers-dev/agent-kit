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
  EscalationState,
  LoopDetection,
  LoopDetectorState,
  StallDetectorState,
} from '../../domain/watchdog/index.js'

import { terminateActiveProcess } from './process-group-control.js'
import {
  isPromiseLike,
  joinPrompt,
  normalizeWatchdogConfig,
  spawnInput,
  thenMaybe,
} from './session-support.js'
import type {
  ActiveProcess,
  InjectDelivery,
  MaybePromise,
  SpawnInput,
  WatchdogConfig,
  WorkerSupervisorDetection,
  WorkerSupervisorResult,
  WorkerSupervisorRuntime,
  WorkerSupervisorSession,
  WorkerSupervisorStartRequest,
  WorkerSupervisorStatus,
} from './types.js'
import { optional } from './types.js'

export class RunningWorker implements WorkerSupervisorSession {
  readonly result: Promise<WorkerSupervisorResult>
  private readonly supervisor: WorkerSupervisorRuntime
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

  constructor(supervisor: WorkerSupervisorRuntime, request: WorkerSupervisorStartRequest) {
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
    return terminateActiveProcess(active, signal, this.supervisor, this.killGraceMs)
  }

  private isFinished(): boolean {
    return this.finished
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
