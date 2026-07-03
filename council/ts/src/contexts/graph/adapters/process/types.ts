import type { ChildProcess, SpawnOptions } from 'node:child_process'

import type {
  DiskUsageCapDetection,
  LoopDetection,
  LoopDetectorConfig,
  LoopDetectorState,
  RetryPolicyState,
  StallDetection,
  WatchdogBudgetDetection,
  WatchdogProgressState,
} from '../../../watchdog/index.js'

export type WorkerSupervisorStatus =
  | 'completed'
  | 'dead-snapshot'
  | 'failed'
  | 'stale-snapshot'
  | 'stopped'
  | 'stalled'
  | 'budget-cap'
  | 'disk-cap'
export type WorkerSupervisorDetection =
  | StallDetection
  | LoopDetection
  | WatchdogBudgetDetection
  | DiskUsageCapDetection
export type InjectDeliveryMode = 'streaming-stdin' | 'checkpoint-and-resume'

export interface WorkerSupervisorWatchdogConfig {
  readonly stallAfterS?: number
  readonly windowSize?: number
  readonly repeatLimit?: number
  readonly maxCycleGram?: number
  readonly maxRestarts?: number
  readonly enableTierEscalation?: boolean
  readonly diskCapBytes?: number
  readonly wallClockCapMs?: number
  readonly outputCapBytes?: number
  readonly attemptTimeoutMs?: number
  readonly retryBaseBackoffMs?: number
  readonly retryMaxBackoffMs?: number
  readonly retryJitterRatio?: number
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
  readonly mcpProfile?: string
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
  readonly stdoutBytes: number
  readonly stderrBytes: number
  readonly stdoutLogPath: string
  readonly stderrLogPath: string
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

export interface WorkerSupervisorEventContext {
  readonly taskId: string
  readonly attemptId: number
  readonly restartCount: number
  readonly pid?: number
  readonly modelTier?: string
  readonly detection?: WorkerSupervisorDetection
}

export type WorkerSupervisorEventDetail =
  | { readonly type: 'started'; readonly restart: number }
  | {
      readonly type: 'stdout'
      readonly tail: string
      readonly offset: number
      readonly byteCount: number
      readonly tailBytes: number
      readonly logPath: string
    }
  | {
      readonly type: 'stderr'
      readonly tail: string
      readonly offset: number
      readonly byteCount: number
      readonly tailBytes: number
      readonly logPath: string
    }
  | { readonly type: 'detected'; readonly detection: WorkerSupervisorDetection }
  | { readonly type: 'terminated'; readonly signal: NodeJS.Signals }
  | { readonly type: 'restarted'; readonly restart: number; readonly previousPid?: number; readonly preamble: string }
  | { readonly type: 'tier-escalated'; readonly modelTier: string }
  | { readonly type: 'injected'; readonly mode: InjectDeliveryMode }
  | { readonly type: 'exited'; readonly exitCode: number | null; readonly signal: NodeJS.Signals | null }
  | { readonly type: 'stopped'; readonly reason: string }

export type WorkerSupervisorEvent = WorkerSupervisorEventContext & WorkerSupervisorEventDetail

export type WorkerSupervisorRuntimeEvent =
  | WorkerSupervisorEvent
  | { readonly type: 'terminated'; readonly signal: NodeJS.Signals; readonly pid: number | undefined }

export type WorkerSupervisorSnapshotStatus =
  | 'running'
  | 'detected'
  | 'restarting'
  | 'exited'
  | 'stopped'
  | 'completed'
  | 'failed'
  | 'stalled'
  | 'budget-cap'
  | 'disk-cap'

export interface WorkerSupervisorSnapshotOffsets {
  readonly stdout: number
  readonly stderr: number
}

export interface WorkerSupervisorSnapshotLogs {
  readonly stdout: string
  readonly stderr: string
}

export interface WorkerSupervisorWatchdogSnapshot {
  readonly progress: WatchdogProgressState
  readonly loop: LoopDetectorState
  readonly retry: RetryPolicyState
  readonly pending_detection?: WorkerSupervisorDetection
  readonly handling_detection: boolean
}

export interface WorkerSupervisorSnapshot {
  readonly task_id: string
  readonly attempt_id: number
  readonly pid?: number
  readonly restart_count: number
  readonly model_tier?: string
  readonly status: WorkerSupervisorSnapshotStatus
  readonly offsets: WorkerSupervisorSnapshotOffsets
  readonly logs: WorkerSupervisorSnapshotLogs
  readonly watchdog: WorkerSupervisorWatchdogSnapshot
  readonly exit_code?: number | null
  readonly signal?: NodeJS.Signals | null
}

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
  readonly onSnapshot?: (snapshot: WorkerSupervisorSnapshot) => MaybePromise<void>
  readonly isPidAlive?: (pid: number) => boolean
  readonly readLogFile?: (path: string) => Promise<Buffer>
}

export interface ActiveProcess {
  readonly child: ChildProcess
  readonly attemptId: number
  readonly restartCount: number
  readonly modelTier?: string
  detection?: WorkerSupervisorDetection
  exited: boolean
  exitCode: number | null
  signal: NodeJS.Signals | null
  terminating: boolean
  exitedPromise: Promise<void>
}

export interface SpawnInput {
  readonly preamble?: string
  readonly modelTier?: string
  readonly detection?: WorkerSupervisorDetection
}

export interface WatchdogConfig {
  readonly stallAfterS: number
  readonly loop: LoopDetectorConfig
  readonly maxRestarts: number
  readonly enableTierEscalation: boolean
  readonly diskCapBytes?: number
  readonly wallClockCapMs?: number
  readonly outputCapBytes?: number
  readonly attemptTimeoutMs?: number
  readonly retryBaseBackoffMs?: number
  readonly retryMaxBackoffMs?: number
  readonly retryJitterRatio?: number
}

export interface WorkerSupervisorRuntime {
  createChild(request: WorkerSupervisorStartRequest, run: number, input: SpawnInput): ChildProcess
  emit(event: WorkerSupervisorRuntimeEvent): void
  snapshot(snapshot: WorkerSupervisorSnapshot): MaybePromise<void>
  now(): number
  sleep(ms: number): Promise<void>
  diskUsageBytes(path: string): Promise<number>
  killGroup(pid: number, signal: NodeJS.Signals): void
  isPidAlive(pid: number): boolean
  readLogFile(path: string): Promise<Buffer>
}

export type MaybePromise<T> = T | Promise<T>

export function optional<Key extends string, Value>(
  key: Key,
  value: Value | undefined,
): Partial<Record<Key, NonNullable<Value>>> {
  return (value === undefined ? {} : { [key]: value }) as Partial<Record<Key, NonNullable<Value>>>
}
