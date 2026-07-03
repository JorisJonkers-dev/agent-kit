import type { EngineDef } from '../../domain/engines/index.js'
import type { JsonRecord } from '../../domain/contracts/index.js'

export interface EngineRunRequest {
  readonly engine: EngineDef
  readonly model: string
  readonly effort: string
  readonly prompt: string
  readonly promptFile: string
  readonly outputFile: string
  readonly cwd?: string
}

export interface EngineSpawnCommand {
  readonly command: string
  readonly args: readonly string[]
  readonly cwd?: string
}

export interface EngineProcessExit {
  readonly exitCode: number
}

export interface EngineChild {
  readonly stdout: AsyncIterable<string>
  readonly stderr: AsyncIterable<string>
  readonly exit: Promise<EngineProcessExit>
  writeStdin(chunk: string): Promise<void>
  closeStdin(): Promise<void>
}

export interface EngineProcess {
  spawn(command: EngineSpawnCommand): EngineChild
}

export interface EngineFileStore {
  writeText(path: string, data: string): Promise<void>
  readText(path: string): Promise<string>
}

export type EngineInjectionMode = 'live_stdin' | 'checkpoint_resume'

export interface EngineInjectionCapability {
  readonly mode: EngineInjectionMode
  readonly evidence: string
}

export interface EngineInjectionReceipt {
  readonly accepted: boolean
  readonly mode: EngineInjectionMode
}

export type EngineEvent =
  | {
      readonly type: 'started'
      readonly engine: string
      readonly command: EngineSpawnCommand
      readonly injection: EngineInjectionCapability
    }
  | {
      readonly type: 'progress'
      readonly stream: 'stdout' | 'stderr'
      readonly text: string
      readonly raw?: JsonRecord
    }
  | {
      readonly type: 'result'
      readonly text: string
      readonly exitCode: number
      readonly costUsd?: number
      readonly metadata?: JsonRecord
    }
  | {
      readonly type: 'failed'
      readonly exitCode: number
      readonly error: string
      readonly stdout: string
      readonly stderr: string
    }

export interface EngineEventStream extends AsyncIterable<EngineEvent> {
  readonly command: EngineSpawnCommand
  readonly injection: EngineInjectionCapability
  inject(prompt: string): Promise<EngineInjectionReceipt>
  closeInput(): Promise<void>
}

export interface EngineAdapterPorts {
  readonly process: EngineProcess
  readonly files: EngineFileStore
}
