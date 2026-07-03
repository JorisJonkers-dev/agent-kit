import type { ChildProcess, SpawnOptions } from 'node:child_process'

import type {
  DiskUsageCapDetection,
  LoopDetection,
  LoopDetectorConfig,
  StallDetection,
} from '../../domain/watchdog/index.js'

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

export interface ActiveProcess {
  readonly child: ChildProcess
  exited: boolean
  exitCode: number | null
  signal: NodeJS.Signals | null
  terminating: boolean
  exitedPromise: Promise<void>
}

export interface SpawnInput {
  readonly preamble?: string
  readonly modelTier?: string
}

export interface WatchdogConfig {
  readonly stallAfterS: number
  readonly loop: LoopDetectorConfig
  readonly maxRestarts: number
  readonly enableTierEscalation: boolean
  readonly diskCapBytes?: number
}

export interface WorkerSupervisorRuntime {
  createChild(request: WorkerSupervisorStartRequest, run: number, input: SpawnInput): ChildProcess
  emit(event: WorkerSupervisorEvent): void
  now(): number
  sleep(ms: number): Promise<void>
  diskUsageBytes(path: string): Promise<number>
  killGroup(pid: number, signal: NodeJS.Signals): void
}

export type MaybePromise<T> = T | Promise<T>

export function optional<Key extends string, Value>(
  key: Key,
  value: Value | undefined,
): Partial<Record<Key, NonNullable<Value>>> {
  return (value === undefined ? {} : { [key]: value }) as Partial<Record<Key, NonNullable<Value>>>
}
