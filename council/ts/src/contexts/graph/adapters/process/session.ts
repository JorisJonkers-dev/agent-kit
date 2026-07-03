import {
  advanceEscalation,
  appendLoopLine,
  createEscalationState,
  createLoopDetectorState,
  createStallDetectorState,
  evaluateDiskUsageCap,
  evaluateStall,
} from '../../../watchdog/index.js'
import type {
  EscalationState,
  LoopDetection,
  LoopDetectorState,
  StallDetectorState,
} from '../../../watchdog/index.js'

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
  WorkerSupervisorEventContext,
  WorkerSupervisorEventDetail,
  WorkerSupervisorDetection,
  WorkerSupervisorResult,
  WorkerSupervisorRuntime,
  WorkerSupervisorRuntimeEvent,
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
  private stdoutBytes = 0
  private stderrBytes = 0
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
      this.emit({ mode: 'streaming-stdin', type: 'injected' })
      return { mode: 'streaming-stdin', restarted: false }
    }

    const preamble = joinPrompt(this.request.checkpointPreamble ?? 'Resume from checkpoint.', turn) ?? ''
    await this.restartWithPreamble(preamble, this.modelTier)
    this.emit({ mode: 'checkpoint-and-resume', type: 'injected' })
    return { mode: 'checkpoint-and-resume', restarted: true }
  }

  async stop(reason = 'requested'): Promise<void> {
    if (!this.finished) {
      await this.terminate(this.active, 'SIGTERM')
      this.finish('stopped', null, null, undefined)
      this.emit({ reason, type: 'stopped' })
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
      this.emit({ detection, type: 'detected' }, this.active, detection)

      const handled =
        detection.kind === 'disk-cap'
          ? thenMaybe(this.terminate(this.active, 'SIGTERM', detection), () => {
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
      return thenMaybe(this.terminate(this.active, 'SIGTERM', detection), () => {
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
      this.emit({ modelTier: this.modelTier, type: 'tier-escalated' }, this.active, detection)
      return this.restartWithPreamble(this.restartPreamble(detection), this.modelTier, detection)
    }

    return thenMaybe(this.terminate(this.active, 'SIGTERM', detection), () => {
      this.finish('stalled', null, null, detection)
    })
  }

  private restartWithPreamble(
    preamble: string,
    modelTier: string | undefined,
    detection?: WorkerSupervisorDetection,
  ): MaybePromise<void> {
    const previousPid = this.active.child.pid
    return thenMaybe(this.terminate(this.active, 'SIGTERM', detection), () => {
      if (this.restarts >= this.watchdog.maxRestarts) {
        this.finish('stalled', null, null, detection)
        return
      }

      this.restarts += 1
      this.pendingLoopDetection = null
      this.stallState = createStallDetectorState(this.supervisor.now(), this.logBytes)
      this.active = this.spawn({
        ...spawnInput(preamble, modelTier),
        ...optional('detection', detection),
      })
      this.emit({
        preamble,
        ...optional('previousPid', previousPid),
        restart: this.restarts,
        type: 'restarted',
      }, this.active, detection)
    })
  }

  private spawn(input: SpawnInput): ActiveProcess {
    this.run += 1
    const run = this.run
    const child = this.supervisor.createChild(this.request, run, input)
    const active: ActiveProcess = {
      attemptId: run,
      child,
      ...optional('detection', input.detection),
      exitCode: null,
      exited: false,
      exitedPromise: Promise.resolve(),
      ...optional('modelTier', input.modelTier),
      restartCount: this.restarts,
      signal: null,
      terminating: false,
    }
    active.exitedPromise = new Promise<void>((resolve) => {
      child.once('exit', (exitCode: number | null, signal: NodeJS.Signals | null) => {
        active.exited = true
        active.exitCode = exitCode
        active.signal = signal
        this.emit({ exitCode, signal, type: 'exited' }, active, active.detection)
        if (this.active === active && !this.finished && !active.terminating) {
          this.finish(exitCode === 0 ? 'completed' : 'failed', exitCode, signal, undefined)
        }
        resolve()
      })
    })
    child.stdout?.on('data', (chunk: Buffer | string) => { this.recordOutput(active, 'stdout', chunk); })
    child.stderr?.on('data', (chunk: Buffer | string) => { this.recordOutput(active, 'stderr', chunk); })
    return active
  }

  private recordOutput(active: ActiveProcess, stream: 'stdout' | 'stderr', chunk: Buffer | string): void {
    const text = chunk.toString()
    const byteCount = Buffer.byteLength(text)
    this.logBytes += byteCount
    if (stream === 'stdout') {
      const offset = this.stdoutBytes
      this.stdout += text
      this.stdoutBytes += byteCount
      this.emit({ byteCount, chunk: text, offset, type: 'stdout' }, active, active.detection)
    } else {
      const offset = this.stderrBytes
      this.stderr += text
      this.stderrBytes += byteCount
      this.emit({ byteCount, chunk: text, offset, type: 'stderr' }, active, active.detection)
    }

    for (const line of text.split(/\r?\n/u)) {
      const loop = appendLoopLine(this.loopState, line, this.watchdog.loop)
      this.loopState = loop.state
      this.pendingLoopDetection = this.pendingLoopDetection ?? loop.detection
    }
  }

  private terminate(
    active: ActiveProcess,
    signal: NodeJS.Signals,
    detection?: WorkerSupervisorDetection,
  ): MaybePromise<void> {
    if (detection !== undefined) {
      active.detection = detection
    }
    return terminateActiveProcess(active, signal, this.terminationRuntime(active, detection), this.killGraceMs)
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

  private terminationRuntime(
    active: ActiveProcess,
    detection: WorkerSupervisorDetection | undefined,
  ): WorkerSupervisorRuntime {
    const emitTerminated = (event: WorkerSupervisorRuntimeEvent): void => {
      const terminated = event as Extract<WorkerSupervisorRuntimeEvent, { readonly type: 'terminated' }>
      this.emit({ signal: terminated.signal, type: 'terminated' }, active, detection ?? active.detection)
    }
    return {
      createChild: this.supervisor.createChild.bind(this.supervisor),
      diskUsageBytes: this.supervisor.diskUsageBytes.bind(this.supervisor),
      emit: emitTerminated,
      killGroup: this.supervisor.killGroup.bind(this.supervisor),
      now: this.supervisor.now.bind(this.supervisor),
      sleep: this.supervisor.sleep.bind(this.supervisor),
    }
  }

  private emit(
    detail: WorkerSupervisorEventDetail,
    active: ActiveProcess = this.active,
    detection: WorkerSupervisorDetection | undefined = active.detection,
  ): void {
    this.supervisor.emit({
      ...this.eventContext(active, detection),
      ...detail,
    })
  }

  private eventContext(
    active: ActiveProcess,
    detection: WorkerSupervisorDetection | undefined,
  ): WorkerSupervisorEventContext {
    return {
      attemptId: active.attemptId,
      ...optional('detection', detection),
      ...optional('modelTier', active.modelTier),
      ...optional('pid', active.child.pid),
      restartCount: active.restartCount,
      taskId: this.request.id,
    }
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
