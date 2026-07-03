import type { EngineInjectionCapability, EngineInjectionReceipt } from './types.js'

export const CLAUDE_STDIN_INJECTION: EngineInjectionCapability = {
  mode: 'live_stdin',
  evidence:
    'local claude -p --help documents --input-format stream-json as realtime streaming input',
}

export const CHECKPOINT_RESUME_INJECTION: EngineInjectionCapability = {
  mode: 'checkpoint_resume',
  evidence: 'v1 non-Claude drivers accept new guidance by restarting from checkpointed state',
}

export function checkpointResumeInjection(): Promise<EngineInjectionReceipt> {
  return Promise.resolve({ accepted: false, mode: 'checkpoint_resume' })
}
