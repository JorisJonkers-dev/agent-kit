import type { EngineDef, JsonRecord } from '../shared-kernel/index.js'

export interface EngineRunRequest {
  readonly engine: EngineDef
  readonly prompt: string
  readonly cwd?: string
  readonly timeoutMs?: number
  readonly retries?: number
}

export interface EngineRunResult {
  readonly stdout: string
  readonly stderr: string
  readonly exitCode: number
  readonly costUsd?: number
  readonly metadata?: JsonRecord
}

export interface EnginePort {
  run(request: EngineRunRequest): Promise<EngineRunResult>
}
